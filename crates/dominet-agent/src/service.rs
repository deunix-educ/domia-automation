// SPDX-License-Identifier: AGPL-3.0-or-later
//! The assistant as the interface uses it (R12 lot A2): one conversation per administrator, a turn that runs in the
//! background, and the cards waiting for a click.
//!
//! The page asks for the new events every second or so ([`AgentHub::poll`]); nothing is pushed. A turn never changes anything:
//! an action becomes a card, and only [`AgentHub::confirm`] (the user's click) runs it, with the user's own account.
//!
//! State lives in memory: a restart empties the conversations and drops the cards, which is the safe side (a card is never
//! run after a restart, nor late: it expires after [`CARD_TTL`]).

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use dominet_domain::mcp::{Actor, AuditKind, NewAuditEntry, redact};
use dominet_domain::repo::McpAuditRepo;
use dominet_domain::user::{Role, User};
use dominet_server::AppState;
use dominet_server::mcp_tokens::{begin_audit, finish_audit};
use serde_json::{Value, json};

use crate::agent::{Agent, Limits};
use crate::bridge::{ActionCard, Bridge, Outcome};
use crate::changes::{self, Undo};
use crate::llm::{Message, OpenAiCompat};
use crate::profile::Regime;

/// Settings of the assistant (administration, category `Assistant`).
pub const SETTING_ENABLED: &str = "AGENT_ENABLED";
pub const SETTING_URL: &str = "AGENT_LLM_URL";
pub const SETTING_MODEL: &str = "AGENT_MODEL";
pub const SETTING_MODEL_ADMIN: &str = "AGENT_MODEL_ADMIN";

/// A card not clicked within this time is dropped: the situation it described may have changed.
pub const CARD_TTL: Duration = Duration::from_secs(15 * 60);
/// Messages kept in a conversation (the oldest go first; a small model has a small window).
const MAX_HISTORY: usize = 40;
/// Longest message accepted from the user.
pub const MAX_INPUT_CHARS: usize = 2000;
/// Events kept per conversation.
const MAX_EVENTS: usize = 400;

/// What the page shows, in order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    /// The user's own message (so that a second tab sees it).
    User(String),
    /// The assistant is working (`regard des appareils...`).
    Status(String),
    /// The assistant's answer.
    Reply(String),
    /// An action waiting for a click.
    Card {
        id: u64,
        tool: String,
        /// The arguments, as compact JSON.
        arguments: String,
        /// What the server says it would do.
        summary: String,
        /// What changes, one plain sentence per field (configuration cards).
        lines: Vec<String>,
        /// The YAML of an automation before and after, one `+ `, `- ` or two-space line each (automations only).
        diff: Vec<String>,
    },
    /// A confirmed card ran: what the tool answered.
    Done { id: u64, text: String },
    /// A change was taken back.
    Undone(String),
    /// A card the user turned down, or that expired.
    Dropped { id: u64, reason: String },
    /// Something failed (LLM unreachable, deadline, refused action).
    Failed(String),
    /// The turn is over.
    Idle,
}

/// The answer to a poll.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Polled {
    pub events: Vec<Event>,
    /// Value to give as `since` next time.
    pub next: usize,
    /// A turn is running.
    pub busy: bool,
    /// The page was ahead of the server (restart, new conversation): it must forget what it showed and start again.
    pub reset: bool,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum HubError {
    #[error("l'assistant est desactive (reglage AGENT_ENABLED)")]
    Disabled,
    #[error("l'assistant est reserve aux administrateurs")]
    NotAdmin,
    #[error("adresse du modele non renseignee (reglage AGENT_LLM_URL)")]
    NoUrl,
    #[error("une demande est deja en cours")]
    Busy,
    #[error("message vide ou trop long (2000 caracteres au plus)")]
    BadInput,
    #[error("cette action n'existe plus (deja traitee ou expiree)")]
    NoSuchCard,
    #[error("cette modification n'existe plus ou ne peut pas etre annulee")]
    NoSuchChange,
}

struct Pending {
    card: ActionCard,
    regime: Regime,
    created: Instant,
    /// What the administrator asked, in clear (for the audit trail).
    request: String,
}

#[derive(Default)]
struct Conversation {
    history: Vec<Message>,
    events: Vec<Event>,
    /// Events already dropped from the front of `events` (so that `since` keeps its meaning).
    dropped: usize,
    busy: bool,
    cards: HashMap<u64, Pending>,
    next_card: u64,
}

impl Conversation {
    fn push(&mut self, e: Event) {
        self.events.push(e);
        if self.events.len() > MAX_EVENTS {
            let extra = self.events.len() - MAX_EVENTS;
            self.events.drain(..extra);
            self.dropped += extra;
        }
    }

    fn trim_history(&mut self) {
        if self.history.len() > MAX_HISTORY {
            let mut cut = self.history.len() - MAX_HISTORY;
            // Never start on a tool result or on an assistant message that asked for tools: their other half is gone.
            while cut < self.history.len() && !matches!(self.history[cut], Message::User(_)) {
                cut += 1;
            }
            self.history.drain(..cut);
        }
    }
}

/// The conversations of the administrators.
#[derive(Default)]
pub struct AgentHub {
    conversations: Mutex<HashMap<i64, Conversation>>,
}

/// The one hub of the process.
#[must_use]
pub fn hub() -> Arc<AgentHub> {
    static HUB: OnceLock<Arc<AgentHub>> = OnceLock::new();
    HUB.get_or_init(|| Arc::new(AgentHub::default())).clone()
}

struct Settings {
    enabled: bool,
    url: String,
    model: String,
    model_admin: String,
}

async fn settings(state: &AppState) -> Settings {
    let get =
        |k: &'static str| async move { state.settings.get_string(k).await.unwrap_or_default() };
    Settings {
        enabled: matches!(get(SETTING_ENABLED).await.as_str(), "true" | "True" | "1"),
        url: get(SETTING_URL).await.trim().to_string(),
        model: get(SETTING_MODEL).await.trim().to_string(),
        model_admin: get(SETTING_MODEL_ADMIN).await.trim().to_string(),
    }
}

fn actor(user: &User) -> Actor {
    Actor::Session {
        user: user.id,
        username: user.username.clone(),
        role: user.role,
    }
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

impl AgentHub {
    /// Whether the assistant is switched on (for the menu and the page).
    pub async fn enabled(state: &AppState) -> bool {
        settings(state).await.enabled
    }

    /// New events of `user`'s conversation, from `since`.
    #[must_use]
    pub fn poll(&self, user: &User, since: usize) -> Polled {
        let mut all = lock(&self.conversations);
        let conv = all.entry(user.id.0).or_default();
        expire(conv);
        let total = conv.dropped + conv.events.len();
        let reset = since > total;
        let from = if reset {
            0
        } else {
            since.saturating_sub(conv.dropped).min(conv.events.len())
        };
        Polled {
            events: conv.events[from..].to_vec(),
            next: total,
            busy: conv.busy,
            reset,
        }
    }

    /// Starts a turn in the background. The answer comes through [`AgentHub::poll`].
    ///
    /// # Errors
    /// See [`HubError`].
    pub async fn send(
        self: &Arc<Self>,
        state: &AppState,
        user: &User,
        text: &str,
        regime: Regime,
    ) -> Result<(), HubError> {
        if user.role != Role::Admin {
            return Err(HubError::NotAdmin);
        }
        let text = text.trim();
        if text.is_empty() || text.chars().count() > MAX_INPUT_CHARS {
            return Err(HubError::BadInput);
        }
        let cfg = settings(state).await;
        if !cfg.enabled {
            return Err(HubError::Disabled);
        }
        if cfg.url.is_empty() {
            return Err(HubError::NoUrl);
        }
        let model = match regime {
            Regime::Daily => cfg.model,
            Regime::Config if cfg.model_admin.is_empty() => cfg.model,
            Regime::Config => cfg.model_admin,
        };
        let history = {
            let mut all = lock(&self.conversations);
            let conv = all.entry(user.id.0).or_default();
            expire(conv);
            if conv.busy {
                return Err(HubError::Busy);
            }
            conv.busy = true;
            conv.push(Event::User(text.to_string()));
            conv.history.clone()
        };
        let hub = Arc::clone(self);
        let state = state.clone();
        let who = actor(user);
        let user_id = user.id.0;
        let text = text.to_string();
        tokio::spawn(async move {
            let outcome = run_turn(&state, who, regime, &cfg.url, &model, &history, &text).await;
            hub.finish(user_id, regime, &text, outcome);
        });
        Ok(())
    }

    /// Puts a card in front of the user (event and pending entry).
    fn offer(conv: &mut Conversation, card: ActionCard, regime: Regime, request: &str) -> u64 {
        conv.next_card += 1;
        let id = conv.next_card;
        let view = crate::diff::view(&card);
        conv.push(Event::Card {
            id,
            tool: card.tool.clone(),
            arguments: card.arguments.to_string(),
            summary: card.summary.clone(),
            lines: view.lines,
            diff: view
                .diff
                .iter()
                .map(crate::diff::DiffLine::render)
                .collect(),
        });
        conv.cards.insert(
            id,
            Pending {
                card,
                regime,
                created: Instant::now(),
                request: request.to_string(),
            },
        );
        id
    }

    /// Registers a card for `user` as if the model had proposed it (the click is still needed to run it). For tests and tools.
    #[doc(hidden)]
    pub fn propose(&self, user: &User, card: ActionCard, regime: Regime, request: &str) -> u64 {
        let mut all = lock(&self.conversations);
        Self::offer(all.entry(user.id.0).or_default(), card, regime, request)
    }

    fn finish(
        &self,
        user_id: i64,
        regime: Regime,
        text: &str,
        outcome: Result<crate::agent::Turn, String>,
    ) {
        let mut all = lock(&self.conversations);
        let conv = all.entry(user_id).or_default();
        match outcome {
            Ok(turn) => {
                // `turn.messages` starts with the user's own message.
                conv.history.extend(turn.messages);
                conv.trim_history();
                if !turn.reply.trim().is_empty() {
                    conv.push(Event::Reply(turn.reply));
                }
                for card in turn.cards {
                    Self::offer(conv, card, regime, text);
                }
            }
            Err(message) => conv.push(Event::Failed(message)),
        }
        conv.busy = false;
        conv.push(Event::Idle);
    }

    /// Runs a card: the user clicked "Confirm". The card is single-use.
    ///
    /// # Errors
    /// [`HubError::NoSuchCard`] if it was handled or expired; the tool's own failure comes back as an event.
    pub async fn confirm(
        &self,
        state: &AppState,
        user: &User,
        card_id: u64,
    ) -> Result<(), HubError> {
        if user.role != Role::Admin {
            return Err(HubError::NotAdmin);
        }
        let pending = {
            let mut all = lock(&self.conversations);
            let conv = all.entry(user.id.0).or_default();
            expire(conv);
            conv.cards.remove(&card_id).ok_or(HubError::NoSuchCard)?
        };
        let result = run_confirmed(state, user, &pending).await;
        let mut all = lock(&self.conversations);
        let conv = all.entry(user.id.0).or_default();
        match result {
            Ok(text) => {
                conv.history.push(Message::User(format!(
                    "[Action confirmee et executee : {} {}] Resultat : {}",
                    pending.card.tool,
                    pending.card.arguments,
                    short(&text, 500)
                )));
                conv.push(Event::Done { id: card_id, text });
            }
            Err(message) => conv.push(Event::Failed(message)),
        }
        Ok(())
    }

    /// The user turned a card down.
    ///
    /// # Errors
    /// [`HubError::NoSuchCard`].
    pub async fn cancel(
        &self,
        state: &AppState,
        user: &User,
        card_id: u64,
    ) -> Result<(), HubError> {
        let pending = {
            let mut all = lock(&self.conversations);
            let conv = all.entry(user.id.0).or_default();
            conv.cards.remove(&card_id).ok_or(HubError::NoSuchCard)?
        };
        let entry = NewAuditEntry {
            kind: AuditKind::Agent,
            actor: user.username.clone(),
            actor_kind: "agent".to_string(),
            token_id: None,
            tool: pending.card.tool.clone(),
            params: json!({
                "decision": "cancelled",
                "request": pending.request,
                "label": changes::describe(&pending.card),
                "arguments": redact(&pending.card.arguments),
            }),
            outcome: "ok".to_string(),
            detail: String::new(),
        };
        if let Err(e) = dominet_server::mcp_tokens::record_audit(state, &entry).await {
            tracing::error!(error = %e, "audit log: assistant refusal not saved");
        }
        let mut all = lock(&self.conversations);
        let conv = all.entry(user.id.0).or_default();
        conv.history.push(Message::User(format!(
            "[Action refusee par l'utilisateur, ne pas la refaire : {} {}]",
            pending.card.tool, pending.card.arguments
        )));
        conv.push(Event::Dropped {
            id: card_id,
            reason: "annulee".into(),
        });
        Ok(())
    }

    /// The changes made through the assistant that can still be looked at (and, for most, taken back), newest first.
    pub async fn changes(&self, state: &AppState) -> Vec<changes::ChangeRow> {
        match state
            .store
            .mcp_audit()
            .recent(Some(AuditKind::Agent), 200)
            .await
        {
            Ok(entries) => changes::rows(&entries),
            Err(e) => {
                tracing::error!(error = %e, "audit log: cannot read the assistant's changes");
                Vec::new()
            }
        }
    }

    /// Takes a change back: the administrator clicked "Undo" on the list. The way back was written to the audit trail before the
    /// change; it is run now with the administrator's account, and recorded as a decision of its own.
    ///
    /// # Errors
    /// [`HubError::NoSuchChange`] if the change is gone, already undone or has no way back; a failure of the tool comes back as an
    /// event.
    pub async fn undo(
        &self,
        state: &AppState,
        user: &User,
        change_id: i64,
    ) -> Result<(), HubError> {
        if user.role != Role::Admin {
            return Err(HubError::NotAdmin);
        }
        let row = self
            .changes(state)
            .await
            .into_iter()
            .find(|r| r.id == change_id)
            .ok_or(HubError::NoSuchChange)?;
        let plan = row.plan.clone().ok_or(HubError::NoSuchChange)?;
        let result = match plan.card(row.created_id) {
            Ok(card) => match Bridge::connect(state.clone(), actor(user), Regime::Config).await {
                Ok(bridge) => bridge.confirm(&card).await.map_err(|e| e.to_string()),
                Err(e) => Err(e.to_string()),
            },
            Err(reason) => Err(reason),
        };
        let entry = NewAuditEntry {
            kind: AuditKind::Agent,
            actor: user.username.clone(),
            actor_kind: "agent".to_string(),
            token_id: None,
            tool: plan.tool.clone(),
            params: json!({"decision": if result.is_ok() { "undone" } else { "undo_failed" }, "of": change_id, "label": row.label}),
            outcome: if result.is_ok() { "ok" } else { "error" }.to_string(),
            detail: result.as_ref().err().cloned().unwrap_or_default(),
        };
        if let Err(e) = dominet_server::mcp_tokens::record_audit(state, &entry).await {
            tracing::error!(error = %e, "audit log: undo not saved");
        }
        let mut all = lock(&self.conversations);
        let conv = all.entry(user.id.0).or_default();
        match result {
            Ok(_) => conv.push(Event::Undone(row.label)),
            Err(message) => conv.push(Event::Failed(message)),
        }
        Ok(())
    }

    /// Starts a new conversation (cards included). A turn in progress finishes into the void.
    pub fn reset(&self, user: &User) {
        let mut all = lock(&self.conversations);
        let busy = all.get(&user.id.0).is_some_and(|c| c.busy);
        let dropped = all
            .get(&user.id.0)
            .map_or(0, |c| c.dropped + c.events.len());
        let mut fresh = Conversation {
            busy,
            dropped,
            ..Conversation::default()
        };
        fresh.push(Event::Idle);
        all.insert(user.id.0, fresh);
    }
}

/// Drops the cards that waited too long.
fn expire(conv: &mut Conversation) {
    let old: Vec<u64> = conv
        .cards
        .iter()
        .filter(|(_, p)| p.created.elapsed() > CARD_TTL)
        .map(|(id, _)| *id)
        .collect();
    for id in old {
        conv.cards.remove(&id);
        conv.push(Event::Dropped {
            id,
            reason: "expiree".into(),
        });
    }
}

/// Runs a confirmed card, **with a snapshot and an audit line first**: for a change to the configuration the item is read as it is,
/// what it takes to put it back is written to the audit trail, and only then is the tool run. If the item cannot be read, or the
/// trail cannot be written, nothing is done.
async fn run_confirmed(state: &AppState, user: &User, pending: &Pending) -> Result<String, String> {
    let bridge = Bridge::connect(state.clone(), actor(user), pending.regime)
        .await
        .map_err(|e| e.to_string())?;
    let card = &pending.card;
    let target = changes::target(card);
    let before = match changes::snapshot_ref(&target) {
        Some((kind, item)) => {
            let read = bridge
                .call("config_get", &json!({"kind": kind, "item": item}))
                .await;
            let parsed = match read {
                Ok(Outcome::Data(text)) => serde_json::from_str::<Value>(&text).ok(),
                _ => None,
            };
            if parsed.is_none() {
                return Err("the current state could not be read (snapshot impossible): nothing was changed".into());
            }
            parsed
        }
        None => None,
    };
    let undo = changes::undo_plan(&target, before.as_ref(), card);
    let entry = NewAuditEntry {
        kind: AuditKind::Agent,
        actor: user.username.clone(),
        actor_kind: "agent".to_string(),
        token_id: None,
        tool: card.tool.clone(),
        params: json!({
            "decision": "confirmed",
            "request": pending.request,
            "label": changes::describe(card),
            "arguments": redact(&card.arguments),
            // Not redacted: it is the way back, and it came from `config_get`, which never returns a secret.
            "undo": match &undo {
                Undo::Plan(p) => p.to_json(),
                Undo::Not(reason) => json!({"not": reason}),
            },
        }),
        outcome: "pending".to_string(),
        detail: String::new(),
    };
    let line = begin_audit(state, &entry).await.map_err(|e| {
        tracing::error!(error = %e, "audit log unavailable: assistant action refused");
        "audit log unavailable: the action was not executed".to_string()
    })?;
    let result = bridge.confirm(card).await.map_err(|e| e.to_string());
    let (outcome, detail) = match &result {
        Ok(text) => {
            let detail = match (&target, changes::created_id(text)) {
                (changes::Target::Create { .. }, Some(id)) => json!({"created_id": id}).to_string(),
                _ => short(text, 200),
            };
            ("ok", detail)
        }
        Err(message) => ("error", short(message, 200)),
    };
    if let Err(e) = finish_audit(state, line, outcome, &detail).await {
        tracing::error!(error = %e, "audit log: action outcome not saved");
    }
    result
}

fn short(text: &str, max: usize) -> String {
    let mut s: String = text.chars().take(max).collect();
    if text.chars().count() > max {
        s.push_str("...");
    }
    s
}

async fn run_turn(
    state: &AppState,
    who: Actor,
    regime: Regime,
    url: &str,
    model: &str,
    history: &[Message],
    text: &str,
) -> Result<crate::agent::Turn, String> {
    let limits = match regime {
        Regime::Daily => Limits::default(),
        // The bigger model is slower: more time, same number of calls.
        Regime::Config => Limits {
            deadline: Duration::from_secs(300),
            ..Limits::default()
        },
    };
    let bridge = Bridge::connect(state.clone(), who, regime)
        .await
        .map_err(|e| e.to_string())?;
    let llm = OpenAiCompat::new(url, model, limits.deadline);
    Agent::new(llm, bridge, limits)
        .run(history, text)
        .await
        .map_err(|e| e.to_string())
}

/// Checks that the model server answers (administration, "Test de connexion"): the models it offers, or the reason it does not.
///
/// # Errors
/// The failure, in words.
pub async fn test_connection(url: &str) -> Result<Vec<String>, String> {
    let url = url.trim().trim_end_matches('/');
    if url.is_empty() {
        return Err("address not set".into());
    }
    let http = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .map_err(|e| e.to_string())?;
    let answer = http
        .get(format!("{url}/v1/models"))
        .send()
        .await
        .map_err(|e| format!("unreachable: {e}"))?;
    if !answer.status().is_success() {
        return Err(format!("the server answers {}", answer.status()));
    }
    let body: serde_json::Value = answer.json().await.map_err(|e| e.to_string())?;
    Ok(body
        .get("data")
        .and_then(serde_json::Value::as_array)
        .map(|models| {
            models
                .iter()
                .filter_map(|m| m.get("id").and_then(serde_json::Value::as_str))
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn conv_with(n: usize) -> Conversation {
        let mut c = Conversation::default();
        for i in 0..n {
            c.push(Event::Status(i.to_string()));
        }
        c
    }

    #[test]
    fn events_are_capped_and_since_keeps_its_meaning() {
        let c = conv_with(MAX_EVENTS + 10);
        assert_eq!(c.events.len(), MAX_EVENTS);
        assert_eq!(c.dropped, 10);
    }

    #[test]
    fn history_is_cut_on_a_user_message() {
        let mut c = Conversation::default();
        for i in 0..30 {
            c.history.push(Message::User(format!("q{i}")));
            c.history.push(Message::Assistant {
                content: format!("a{i}"),
                tool_calls: vec![],
            });
        }
        c.trim_history();
        assert!(c.history.len() <= MAX_HISTORY);
        assert!(matches!(c.history[0], Message::User(_)));
    }

    #[test]
    fn a_page_ahead_of_the_server_is_told_to_start_again() {
        let hub = AgentHub::default();
        let user = fake_admin();
        {
            let mut all = lock(&hub.conversations);
            all.entry(user.id.0)
                .or_default()
                .push(Event::Reply("a".into()));
        }
        let polled = hub.poll(&user, 50);
        assert!(polled.reset);
        assert_eq!(polled.events, vec![Event::Reply("a".into())]);
    }

    #[test]
    fn poll_returns_only_what_is_new() {
        let hub = AgentHub::default();
        let user = fake_admin();
        let first = hub.poll(&user, 0);
        assert!(first.events.is_empty());
        {
            let mut all = lock(&hub.conversations);
            all.entry(user.id.0)
                .or_default()
                .push(Event::Reply("a".into()));
        }
        let next = hub.poll(&user, first.next);
        assert_eq!(next.events, vec![Event::Reply("a".into())]);
        assert!(hub.poll(&user, next.next).events.is_empty());
    }

    fn fake_admin() -> User {
        use dominet_domain::user::UserId;
        let now = chrono::Utc::now();
        User {
            id: UserId(7),
            username: "root".into(),
            email: String::new(),
            first_name: String::new(),
            last_name: String::new(),
            mobile: String::new(),
            role: Role::Admin,
            is_active: true,
            last_login_at: None,
            password_changed_at: now,
            created_at: now,
            updated_at: now,
        }
    }
}
