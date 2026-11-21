// SPDX-License-Identifier: AGPL-3.0-or-later
//! MCP access (R11, ADR 0059): scopes, the actor behind a call, access tokens and the audit trail.
//!
//! Pure rules only: how a token is hashed and stored lives in `dominet-server` and `dominet-store`. A token value is never kept
//! here: the domain only knows its public part (name, prefix, scopes, dates).

use std::collections::BTreeSet;
use std::fmt;

use chrono::{DateTime, Duration, Utc};
use serde_json::Value;

use crate::automation::Problems;
use crate::error::Problem;
use crate::user::{Role, UserId};

/// What a token may do. Scopes are independent: `configuration` does not imply `lecture`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Scope {
    /// Look: devices, history, logs, alerts, automations, configuration (secrets masked).
    Read,
    /// Act on the house: commands, scenes, PTZ, acknowledging alerts.
    Control,
    /// Change the configuration: automations, rules, dashboards, sources.
    Configuration,
    /// Sensitive operations: settings, cameras and their credentials, Rhai scripts.
    Administration,
}

impl Scope {
    pub const ALL: [Scope; 4] = [
        Scope::Read,
        Scope::Control,
        Scope::Configuration,
        Scope::Administration,
    ];

    /// Name stored in the database and shown to the user.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Scope::Read => "lecture",
            Scope::Control => "pilotage",
            Scope::Configuration => "configuration",
            Scope::Administration => "administration",
        }
    }

    #[must_use]
    pub fn parse(name: &str) -> Option<Scope> {
        Scope::ALL.into_iter().find(|s| s.as_str() == name)
    }
}

impl fmt::Display for Scope {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

pub type Scopes = BTreeSet<Scope>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct McpTokenId(pub i64);

/// Who is behind a call: an access token, or a signed-in web session (the built-in assistant of R12 has no token).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Actor {
    Token {
        id: McpTokenId,
        name: String,
        scopes: Scopes,
        /// The administrator who created the token: the account an action done "by" the token is attributed to (acknowledging
        /// an alert records an account). `None` once that account is deleted: such a token cannot do those actions.
        owner: Option<UserId>,
    },
    Session {
        user: UserId,
        username: String,
        role: Role,
    },
}

impl Actor {
    /// Scopes the actor holds. A session follows its role: an administrator holds all four, any other account `lecture` and
    /// `pilotage` (what the interface lets it do).
    #[must_use]
    pub fn scopes(&self) -> Scopes {
        match self {
            Actor::Token { scopes, .. } => scopes.clone(),
            Actor::Session { role, .. } => match role {
                Role::Admin => Scope::ALL.into_iter().collect(),
                Role::User => [Scope::Read, Scope::Control].into_iter().collect(),
            },
        }
    }

    #[must_use]
    pub fn allows(&self, scope: Scope) -> bool {
        self.scopes().contains(&scope)
    }

    /// Name written in the audit trail.
    #[must_use]
    pub fn label(&self) -> &str {
        match self {
            Actor::Token { name, .. } => name,
            Actor::Session { username, .. } => username,
        }
    }

    #[must_use]
    pub fn kind(&self) -> &'static str {
        match self {
            Actor::Token { .. } => "token",
            Actor::Session { .. } => "session",
        }
    }

    /// The account an action is attributed to: the owner of a token, the account of a session.
    #[must_use]
    pub fn account(&self) -> Option<UserId> {
        match self {
            Actor::Token { owner, .. } => *owner,
            Actor::Session { user, .. } => Some(*user),
        }
    }

    #[must_use]
    pub fn token_id(&self) -> Option<McpTokenId> {
        match self {
            Actor::Token { id, .. } => Some(*id),
            Actor::Session { .. } => None,
        }
    }
}

/// Longest lifetime of a token, in days (ADR 0059).
pub const MAX_LIFETIME_DAYS: u32 = 365;
/// Lifetime proposed when creating a token.
pub const DEFAULT_LIFETIME_DAYS: u32 = 90;
/// Longest token name.
pub const MAX_NAME_CHARS: usize = 64;

/// A token to create, validated.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewMcpToken {
    pub name: String,
    pub scopes: Scopes,
    pub lifetime_days: u32,
}

impl NewMcpToken {
    /// # Errors
    /// Field by field: `name` (required, 64 characters at most), `scopes` (at least one, known names), `lifetime_days`
    /// (1 to 365: a token without an end date does not exist).
    pub fn new(name: &str, scopes: &[String], lifetime_days: u32) -> Result<Self, Problems> {
        let mut problems: Problems = vec![];
        let name = name.trim();
        if name.is_empty() {
            problems.push(("name", Problem::Required));
        } else if name.chars().count() > MAX_NAME_CHARS {
            problems.push((
                "name",
                Problem::TooLong {
                    max: MAX_NAME_CHARS,
                },
            ));
        }
        let mut set = Scopes::new();
        for s in scopes {
            match Scope::parse(s) {
                Some(scope) => {
                    set.insert(scope);
                }
                None => problems.push(("scopes", Problem::Invalid)),
            }
        }
        if set.is_empty() && !problems.iter().any(|(f, _)| *f == "scopes") {
            problems.push(("scopes", Problem::Required));
        }
        if !(1..=MAX_LIFETIME_DAYS).contains(&lifetime_days) {
            problems.push((
                "lifetime_days",
                Problem::OutOfRange {
                    min: 1,
                    max: i64::from(MAX_LIFETIME_DAYS),
                },
            ));
        }
        if problems.is_empty() {
            Ok(Self {
                name: name.to_string(),
                scopes: set,
                lifetime_days,
            })
        } else {
            Err(problems)
        }
    }

    /// End of validity for a token created at `now`.
    #[must_use]
    pub fn expires_at(&self, now: DateTime<Utc>) -> DateTime<Utc> {
        now + Duration::days(i64::from(self.lifetime_days))
    }
}

/// A stored token, as the administration shows it. The value itself is not recoverable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct McpToken {
    pub id: McpTokenId,
    pub name: String,
    /// First characters of the value (`dnm_ab12cd34`), to recognise a token without being able to use it.
    pub prefix: String,
    pub scopes: Scopes,
    pub created_at: DateTime<Utc>,
    pub created_by: Option<UserId>,
    pub expires_at: DateTime<Utc>,
    pub last_used_at: Option<DateTime<Utc>>,
    pub revoked_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenState {
    Active,
    Expired,
    Revoked,
}

impl McpToken {
    /// A revoked token stays revoked even once its date has passed.
    #[must_use]
    pub fn state(&self, now: DateTime<Utc>) -> TokenState {
        if self.revoked_at.is_some() {
            TokenState::Revoked
        } else if self.expires_at <= now {
            TokenState::Expired
        } else {
            TokenState::Active
        }
    }
}

/// What an audit line is about.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuditKind {
    TokenCreated,
    TokenRevoked,
    /// A tool call that changes something (from the control lot on).
    ToolCall,
    /// A call refused for lack of scope.
    Denied,
    /// A decision about an action the built-in assistant proposed (confirmed, cancelled, undone), with the request in clear
    /// and what is needed to undo it (R12 lot A3).
    Agent,
}

impl AuditKind {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            AuditKind::TokenCreated => "token_created",
            AuditKind::TokenRevoked => "token_revoked",
            AuditKind::ToolCall => "tool_call",
            AuditKind::Denied => "denied",
            AuditKind::Agent => "agent",
        }
    }

    #[must_use]
    pub fn parse(name: &str) -> Option<AuditKind> {
        [
            AuditKind::TokenCreated,
            AuditKind::TokenRevoked,
            AuditKind::ToolCall,
            AuditKind::Denied,
            AuditKind::Agent,
        ]
        .into_iter()
        .find(|k| k.as_str() == name)
    }
}

/// An audit line to write.
#[derive(Debug, Clone, PartialEq)]
pub struct NewAuditEntry {
    pub kind: AuditKind,
    /// Who: the token's name or the account's username.
    pub actor: String,
    /// `token`, `session` or `web` (an administrator in the interface).
    pub actor_kind: String,
    pub token_id: Option<McpTokenId>,
    pub tool: String,
    /// Parameters, secrets masked (see [`redact`]).
    pub params: Value,
    /// `ok`, `error` or `denied`.
    pub outcome: String,
    pub detail: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct McpAuditEntry {
    pub id: i64,
    pub at: DateTime<Utc>,
    pub kind: String,
    pub actor: String,
    pub actor_kind: String,
    pub token_id: Option<McpTokenId>,
    pub tool: String,
    pub params: Value,
    pub outcome: String,
    pub detail: String,
}

/// Longest text kept in an audit line (a parameter or a detail).
const MAX_AUDIT_TEXT: usize = 500;

/// Whether a key names a secret: masked in the audit trail whatever its value. A bare `key` is not one (it is the name of a
/// state key in an alert rule or a condition: `temperature`), `api_key` and the like are.
fn is_secret_key(key: &str) -> bool {
    let k = key.to_ascii_lowercase();
    [
        "password",
        "passwd",
        "secret",
        "token",
        "credential",
        "authorization",
        "api_key",
        "apikey",
        "private_key",
        "master_key",
        "pass",
    ]
    .iter()
    .any(|needle| k.contains(needle))
}

/// Copy of `value` for the audit trail: values of secret-looking keys become `***`, long texts are cut.
#[must_use]
pub fn redact(value: &Value) -> Value {
    match value {
        Value::Object(map) => Value::Object(
            map.iter()
                .map(|(k, v)| {
                    let v = if is_secret_key(k) {
                        Value::String("***".into())
                    } else {
                        redact(v)
                    };
                    (k.clone(), v)
                })
                .collect(),
        ),
        Value::Array(items) => Value::Array(items.iter().map(redact).collect()),
        Value::String(s) if s.chars().count() > MAX_AUDIT_TEXT => {
            Value::String(s.chars().take(MAX_AUDIT_TEXT).chain(['…']).collect())
        }
        other => other.clone(),
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn at(day: u32) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(&format!("2026-11-{day:02}T10:00:00Z"))
            .unwrap()
            .with_timezone(&Utc)
    }

    #[test]
    fn scope_names_round_trip_and_scopes_are_independent() {
        for s in Scope::ALL {
            assert_eq!(Scope::parse(s.as_str()), Some(s));
        }
        assert_eq!(Scope::parse("root"), None);
        let a = Actor::Token {
            id: McpTokenId(1),
            name: "essai".into(),
            scopes: [Scope::Configuration].into_iter().collect(),
            owner: Some(UserId(9)),
        };
        assert!(a.allows(Scope::Configuration));
        // Holding `configuration` grants neither reading nor control.
        assert!(!a.allows(Scope::Read) && !a.allows(Scope::Control));
        assert_eq!(a.account(), Some(UserId(9)));
    }

    #[test]
    fn a_session_follows_its_role() {
        let admin = Actor::Session {
            user: UserId(1),
            username: "alice".into(),
            role: Role::Admin,
        };
        assert!(Scope::ALL.into_iter().all(|s| admin.allows(s)));
        let user = Actor::Session {
            user: UserId(2),
            username: "bob".into(),
            role: Role::User,
        };
        assert!(user.allows(Scope::Read) && user.allows(Scope::Control));
        assert!(!user.allows(Scope::Configuration) && !user.allows(Scope::Administration));
        assert_eq!((admin.kind(), user.label()), ("session", "bob"));
    }

    #[test]
    fn a_new_token_is_validated_field_by_field() {
        let ok =
            NewMcpToken::new("  Claude Code  ", &["lecture".into(), "lecture".into()], 90).unwrap();
        assert_eq!(ok.name, "Claude Code");
        assert_eq!(ok.scopes.len(), 1);
        assert_eq!(ok.expires_at(at(16)), at(16) + Duration::days(90));
        let problems = NewMcpToken::new("", &[], 0).unwrap_err();
        assert_eq!(
            problems,
            [
                ("name", Problem::Required),
                ("scopes", Problem::Required),
                ("lifetime_days", Problem::OutOfRange { min: 1, max: 365 }),
            ]
        );
        assert_eq!(
            NewMcpToken::new("x", &["lecture".into()], 366).unwrap_err(),
            [("lifetime_days", Problem::OutOfRange { min: 1, max: 365 })]
        );
        assert_eq!(
            NewMcpToken::new(&"n".repeat(65), &["lecture".into()], 30).unwrap_err(),
            [("name", Problem::TooLong { max: 64 })]
        );
        assert_eq!(
            NewMcpToken::new("x", &["root".into()], 30).unwrap_err(),
            [("scopes", Problem::Invalid)]
        );
    }

    #[test]
    fn the_state_of_a_token_depends_on_revocation_first_then_the_date() {
        let mut t = McpToken {
            id: McpTokenId(1),
            name: "t".into(),
            prefix: "dnm_ab12".into(),
            scopes: [Scope::Read].into_iter().collect(),
            created_at: at(1),
            created_by: None,
            expires_at: at(20),
            last_used_at: None,
            revoked_at: None,
        };
        assert_eq!(t.state(at(16)), TokenState::Active);
        assert_eq!(t.state(at(20)), TokenState::Expired);
        t.revoked_at = Some(at(10));
        assert_eq!(t.state(at(16)), TokenState::Revoked);
        assert_eq!(t.state(at(25)), TokenState::Revoked);
    }

    #[test]
    fn secrets_are_masked_at_any_depth_and_long_texts_are_cut() {
        let v = json!({
            "name": "Camera salon",
            "rtsp_password": "tres-secret",
            "nested": {"api_token": "abc", "ok": 1, "list": [{"Authorization": "Bearer x", "n": 2}]},
            "long": "x".repeat(600),
        });
        let r = redact(&v);
        assert_eq!(r["name"], "Camera salon");
        assert_eq!(r["rtsp_password"], "***");
        assert_eq!(r["nested"]["api_token"], "***");
        assert_eq!(r["nested"]["ok"], 1);
        assert_eq!(r["nested"]["list"][0]["Authorization"], "***");
        assert_eq!(r["nested"]["list"][0]["n"], 2);
        assert_eq!(r["long"].as_str().unwrap().chars().count(), 501);
        assert!(!r.to_string().contains("tres-secret"));
        // The name of a state key is not a secret; an API key is.
        let r = redact(&json!({"key": "temperature", "web_api_key": "abc", "api_key": "def"}));
        assert_eq!(r["key"], "temperature");
        assert_eq!(
            (&r["web_api_key"], &r["api_key"]),
            (&json!("***"), &json!("***"))
        );
    }

    #[test]
    fn audit_kinds_round_trip() {
        for k in [
            AuditKind::TokenCreated,
            AuditKind::TokenRevoked,
            AuditKind::ToolCall,
            AuditKind::Denied,
        ] {
            assert_eq!(AuditKind::parse(k.as_str()), Some(k));
        }
        assert_eq!(AuditKind::parse("autre"), None);
    }
}
