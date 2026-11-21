// SPDX-License-Identifier: AGPL-3.0-or-later
//! Administration of the MCP access tokens and of the audit trail: `/admin/mcp` (administrators; R11 M1, ADR 0059).
//!
//! A token's value is shown **once**, right after its creation, with the command that plugs it into Claude Code; only its hash
//! is stored. The page never offers to read a value back, and creating or revoking a token is written to the audit trail.

use dominet_proto::{FieldProblem, SaveOutcome};
use leptos::prelude::*;
use leptos_fluent::{move_tr, tr};
use serde::{Deserialize, Serialize};

use crate::browser::{clear_input, input_value};
use crate::crud::field_error;
use crate::guard::RequireAdmin;
use crate::layout::use_hydrated;

/// A token as the list shows it (no secret: the prefix is only the start of the value).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct McpTokenDto {
    pub id: i64,
    pub name: String,
    pub prefix: String,
    pub scopes: Vec<String>,
    pub created_at: String,
    pub expires_at: String,
    /// Empty if never used.
    pub last_used_at: String,
    /// `active`, `expired` or `revoked`.
    pub state: String,
}

/// What the page needs to show: the server state and the tokens.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct McpPageDto {
    pub enabled: bool,
    pub path: String,
    pub rate_limit_per_min: u32,
    pub retention_days: u32,
    pub tokens: Vec<McpTokenDto>,
}

/// The creation form.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct McpTokenInput {
    pub name: String,
    pub scopes: Vec<String>,
    pub days: u32,
}

/// Answer to a creation: the value, **only here**, and the command ready to paste.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CreatedMcpTokenDto {
    pub value: String,
    pub command: String,
    pub token: McpTokenDto,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct McpAuditFilter {
    /// `token_created`, `token_revoked`, `tool_call`, `denied`, `agent`; empty for all.
    pub kind: String,
    pub limit: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct McpAuditDto {
    pub id: i64,
    pub at: String,
    pub kind: String,
    pub actor: String,
    pub actor_kind: String,
    pub tool: String,
    pub outcome: String,
    pub detail: String,
    /// Compact JSON, secrets already masked.
    pub params: String,
}

#[cfg(feature = "ssr")]
mod ssr {
    use chrono::{DateTime, Utc};
    use dominet_domain::mcp::{McpToken, TokenState};
    use dominet_server::timefmt;

    use super::McpTokenDto;

    pub fn token_dto(t: &McpToken, now: DateTime<Utc>) -> McpTokenDto {
        McpTokenDto {
            id: t.id.0,
            name: t.name.clone(),
            prefix: t.prefix.clone(),
            scopes: t.scopes.iter().map(|s| s.as_str().to_string()).collect(),
            created_at: timefmt::text(t.created_at),
            expires_at: timefmt::text(t.expires_at),
            last_used_at: t.last_used_at.map(timefmt::text).unwrap_or_default(),
            state: match t.state(now) {
                TokenState::Active => "active",
                TokenState::Expired => "expired",
                TokenState::Revoked => "revoked",
            }
            .to_string(),
        }
    }

    /// Host to show in the connection command: the first configured name, or a placeholder to fill in.
    pub fn host(allowed: &[String]) -> String {
        allowed
            .iter()
            .find(|h| h.chars().any(|c| c.is_ascii_alphabetic()))
            .or_else(|| allowed.first())
            .cloned()
            .unwrap_or_else(|| "<hote>".to_string())
    }
}

/// State of the server and list of tokens (administrators).
///
/// # Errors
/// Refus (403) pour un non-administrateur ; erreur interne si la base echoue.
#[server(name = McpAdminPage, prefix = "/sf", endpoint = "mcp/page")]
pub async fn mcp_admin_page() -> Result<McpPageDto, ServerFnError> {
    use chrono::Utc;
    use dominet_domain::repo::McpTokenRepo;

    use crate::server_fns::{internal, require_admin};

    let state = expect_context::<dominet_server::AppState>();
    require_admin(&state).await?;
    let now = Utc::now();
    let tokens = state
        .store
        .mcp_tokens()
        .list()
        .await
        .map_err(|e| internal("MCP token list", &e))?;
    Ok(McpPageDto {
        enabled: state.config.mcp_enabled,
        path: "/mcp".to_string(),
        rate_limit_per_min: state.config.mcp_rate_limit_per_min,
        retention_days: state.config.mcp_audit_retention_days,
        tokens: tokens.iter().map(|t| ssr::token_dto(t, now)).collect(),
    })
}

/// Creates a token. Its value is in this answer and nowhere else: it cannot be read back.
///
/// # Errors
/// Refus (403) pour un non-administrateur ; erreur interne si la base echoue.
#[server(name = CreateMcpToken, prefix = "/sf", endpoint = "mcp/tokens/create", input = server_fn::codec::Json)]
pub async fn create_mcp_token(
    input: McpTokenInput,
) -> Result<SaveOutcome<CreatedMcpTokenDto>, ServerFnError> {
    use chrono::Utc;
    use dominet_domain::mcp::{AuditKind, NewAuditEntry, NewMcpToken};
    use dominet_domain::repo::McpTokenRepo;
    use dominet_server::mcp_tokens;
    use serde_json::json;

    use crate::server_fns::{field_problems, internal, require_admin};

    let state = expect_context::<dominet_server::AppState>();
    let admin = require_admin(&state).await?;
    let new = match NewMcpToken::new(&input.name, &input.scopes, input.days) {
        Ok(new) => new,
        Err(problems) => return Ok(SaveOutcome::Invalid(field_problems(problems))),
    };
    let generated = mcp_tokens::generate();
    let now = Utc::now();
    let token = state
        .store
        .mcp_tokens()
        .create(
            &new,
            &generated.hash,
            &generated.prefix,
            now,
            Some(admin.id),
        )
        .await
        .map_err(|e| internal("creating an MCP token", &e))?;
    let entry = NewAuditEntry {
        kind: AuditKind::TokenCreated,
        actor: admin.username.clone(),
        actor_kind: "web".to_string(),
        token_id: Some(token.id),
        tool: String::new(),
        params: json!({
            "name": token.name,
            "scopes": token.scopes.iter().map(|s| s.as_str()).collect::<Vec<_>>(),
            "lifetime_days": new.lifetime_days,
        }),
        outcome: "ok".to_string(),
        detail: String::new(),
    };
    if let Err(e) = mcp_tokens::record_audit(&state, &entry).await {
        tracing::error!(error = %e, "MCP audit log: write failed");
    }
    // The value is never logged: only who created which token.
    tracing::info!(
        admin_id = admin.id.0,
        token_id = token.id.0,
        "MCP token created"
    );
    let host = ssr::host(&state.config.allowed_hosts);
    let command = format!(
        "claude mcp add --transport http dominet https://{host}/mcp --header \"Authorization: Bearer {}\"",
        generated.value
    );
    Ok(SaveOutcome::Saved(CreatedMcpTokenDto {
        command,
        value: generated.value,
        token: ssr::token_dto(&token, now),
    }))
}

/// Revokes a token: refused from the very next call. `false` if it was not active.
///
/// # Errors
/// Refus (403) pour un non-administrateur ; erreur interne si la base echoue.
#[server(name = RevokeMcpToken, prefix = "/sf", endpoint = "mcp/tokens/revoke")]
pub async fn revoke_mcp_token(id: i64) -> Result<bool, ServerFnError> {
    use chrono::Utc;
    use dominet_domain::mcp::{AuditKind, McpTokenId, NewAuditEntry};
    use dominet_domain::repo::McpTokenRepo;
    use dominet_server::mcp_tokens;
    use serde_json::json;

    use crate::server_fns::{internal, require_admin};

    let state = expect_context::<dominet_server::AppState>();
    let admin = require_admin(&state).await?;
    let tokens = state.store.mcp_tokens();
    let name = tokens
        .list()
        .await
        .map_err(|e| internal("MCP token list", &e))?
        .into_iter()
        .find(|t| t.id.0 == id)
        .map(|t| t.name)
        .unwrap_or_default();
    let done = tokens
        .revoke(McpTokenId(id), Utc::now())
        .await
        .map_err(|e| internal("revoking an MCP token", &e))?;
    if done {
        let entry = NewAuditEntry {
            kind: AuditKind::TokenRevoked,
            actor: admin.username.clone(),
            actor_kind: "web".to_string(),
            token_id: Some(McpTokenId(id)),
            tool: String::new(),
            params: json!({"id": id, "name": name}),
            outcome: "ok".to_string(),
            detail: String::new(),
        };
        if let Err(e) = mcp_tokens::record_audit(&state, &entry).await {
            tracing::error!(error = %e, "MCP audit log: write failed");
        }
        tracing::info!(admin_id = admin.id.0, token_id = id, "MCP token revoked");
    }
    Ok(done)
}

/// Audit trail, newest first (`limit` is capped at 500).
///
/// # Errors
/// Refus (403) pour un non-administrateur ; erreur interne si la base echoue.
#[server(name = ListMcpAudit, prefix = "/sf", endpoint = "mcp/audit/list", input = server_fn::codec::Json)]
pub async fn list_mcp_audit(filter: McpAuditFilter) -> Result<Vec<McpAuditDto>, ServerFnError> {
    use dominet_domain::mcp::AuditKind;
    use dominet_domain::repo::McpAuditRepo;
    use dominet_server::timefmt;

    use crate::server_fns::{internal, require_admin};

    let state = expect_context::<dominet_server::AppState>();
    require_admin(&state).await?;
    let limit = i64::from(filter.limit.clamp(1, 500));
    let kind = AuditKind::parse(&filter.kind);
    let lines = state
        .store
        .mcp_audit()
        .recent(kind, limit)
        .await
        .map_err(|e| internal("journal d'audit MCP", &e))?;
    Ok(lines
        .into_iter()
        .map(|l| McpAuditDto {
            id: l.id,
            at: timefmt::text(l.at),
            kind: l.kind,
            actor: l.actor,
            actor_kind: l.actor_kind,
            tool: l.tool,
            outcome: l.outcome,
            detail: l.detail,
            params: if l.params.as_object().is_some_and(serde_json::Map::is_empty) {
                String::new()
            } else {
                l.params.to_string()
            },
        })
        .collect())
}

/// Scope names, translated.
fn scope_text(scope: &str) -> String {
    match scope {
        "lecture" => tr!("mcp-scope-lecture"),
        "pilotage" => tr!("mcp-scope-pilotage"),
        "configuration" => tr!("mcp-scope-configuration"),
        _ => tr!("mcp-scope-administration"),
    }
}

fn state_text(state: &str) -> String {
    match state {
        "active" => tr!("mcp-state-active"),
        "expired" => tr!("mcp-state-expired"),
        _ => tr!("mcp-state-revoked"),
    }
}

fn kind_text(kind: &str) -> String {
    match kind {
        "token_created" => tr!("mcp-kind-created"),
        "token_revoked" => tr!("mcp-kind-revoked"),
        "tool_call" => tr!("mcp-kind-call"),
        "denied" => tr!("mcp-kind-denied"),
        "agent" => tr!("mcp-kind-agent"),
        other => other.to_string(),
    }
}

#[component]
pub fn McpAdminPage() -> impl IntoView {
    view! {
        <RequireAdmin>
            <main class="vc-container">
                <McpAdmin/>
            </main>
        </RequireAdmin>
    }
}

#[component]
fn McpAdmin() -> impl IntoView {
    let version = RwSignal::new(0u32);
    // The value of a token just created: shown once, until hidden or until the page is left.
    let created = RwSignal::new(None::<CreatedMcpTokenDto>);
    let data = Resource::new(
        move || version.get(),
        |_| async { mcp_admin_page().await.map_err(|_| ()) },
    );
    view! {
        <div class="vc-page-header">
            <h2>{move_tr!("mcp-title")}</h2>
        </div>
        <p class="vc-muted">{move_tr!("mcp-intro")}</p>
        <Transition fallback=|| ()>
            {move || match data.get() {
                None => ().into_any(),
                Some(Err(())) => view! { <div class="vc-alert vc-alert--error" role="alert">{move_tr!("crud-load-error")}</div> }.into_any(),
                Some(Ok(page)) => view! {
                    <ServerState page=page.clone()/>
                    <TokenForm version=version created=created/>
                    <CreatedToken created=created/>
                    <TokenTable tokens=page.tokens version=version/>
                }.into_any(),
            }}
        </Transition>
        <AuditTable version=version/>
    }
}

/// Whether the server is on, and its limits.
#[component]
fn ServerState(page: McpPageDto) -> impl IntoView {
    // Les textes sont calcules dans des fermetures : ils suivent le changement de langue sans recharger la page.
    let (rate, days) = (page.rate_limit_per_min, page.retention_days);
    let limits = move || {
        tr!("mcp-limits", {
            "rate" => rate.to_string(),
            "days" => days.to_string()
        })
    };
    let path = page.path.clone();
    let status = if page.enabled {
        view! { <div class="vc-alert vc-alert--success" id="mcp-status" role="status">{move || tr!("mcp-status-on", {"path" => path.clone()})}</div> }.into_any()
    } else {
        view! { <div class="vc-alert vc-alert--warning" id="mcp-status" role="status">{move_tr!("mcp-status-off")}</div> }.into_any()
    };
    view! {
        {status}
        <p class="vc-muted" id="mcp-limits">{limits}</p>
    }
}

/// A checkbox of the scope list.
#[component]
fn ScopeBox(
    scope: &'static str,
    scopes: RwSignal<Vec<String>>,
    children: Children,
) -> impl IntoView {
    let ready = use_hydrated();
    view! {
        <label class="vc-check" data-scope=scope>
            <input
                type="checkbox"
                id=format!("mcp-scope-{scope}")
                prop:checked=move || scopes.with(|s| s.iter().any(|x| x == scope))
                disabled=move || !ready.get()
                on:change=move |ev| {
                    let on = event_target_checked(&ev);
                    scopes.update(|s| {
                        s.retain(|x| x != scope);
                        if on {
                            s.push(scope.to_string());
                        }
                    });
                }
            />
            <span>{children()}</span>
        </label>
    }
}

#[component]
fn TokenForm(
    version: RwSignal<u32>,
    created: RwSignal<Option<CreatedMcpTokenDto>>,
) -> impl IntoView {
    let ready = use_hydrated();
    let name = NodeRef::<leptos::html::Input>::new();
    let scopes = RwSignal::new(vec!["lecture".to_string()]);
    let days = RwSignal::new(90u32);
    let problems = RwSignal::new(Vec::<FieldProblem>::new());
    let create = Action::new(|input: &McpTokenInput| {
        let input = input.clone();
        async move { create_mcp_token(input).await }
    });
    Effect::new(move |_| {
        if let Some(Ok(outcome)) = create.value().get() {
            match outcome {
                SaveOutcome::Saved(token) => {
                    problems.set(vec![]);
                    clear_input(name);
                    created.set(Some(token));
                    version.update(|v| *v += 1);
                }
                SaveOutcome::Invalid(p) => problems.set(p),
                SaveOutcome::NotFound | SaveOutcome::Blocked(_) => {}
            }
        }
    });
    let submit = move |ev: leptos::ev::SubmitEvent| {
        ev.prevent_default();
        create.dispatch(McpTokenInput {
            name: input_value(name),
            scopes: scopes.get_untracked(),
            days: days.get_untracked(),
        });
    };
    view! {
        <div class="vc-card">
            <h3>{move_tr!("mcp-create")}</h3>
            <form on:submit=submit id="mcp-create-form">
                <div class="vc-form-group">
                    <label for="mcp-name">{move_tr!("mcp-name")}</label>
                    <input id="mcp-name" type="text" node_ref=name maxlength="64" placeholder=move || tr!("mcp-name-placeholder")/>
                    {field_error("name", problems)}
                </div>
                <fieldset class="vc-form-group">
                    <legend>{move_tr!("mcp-scopes")}</legend>
                    <ScopeBox scope="lecture" scopes=scopes>
                        <strong>{move_tr!("mcp-scope-lecture")}</strong>" : "{move_tr!("mcp-scope-lecture-hint")}
                    </ScopeBox>
                    <ScopeBox scope="pilotage" scopes=scopes>
                        <strong>{move_tr!("mcp-scope-pilotage")}</strong>" : "{move_tr!("mcp-scope-pilotage-hint")}
                    </ScopeBox>
                    <ScopeBox scope="configuration" scopes=scopes>
                        <strong>{move_tr!("mcp-scope-configuration")}</strong>" : "{move_tr!("mcp-scope-configuration-hint")}
                    </ScopeBox>
                    <ScopeBox scope="administration" scopes=scopes>
                        <strong>{move_tr!("mcp-scope-administration")}</strong>" : "{move_tr!("mcp-scope-administration-hint")}
                    </ScopeBox>
                    {field_error("scopes", problems)}
                </fieldset>
                <div class="vc-form-group">
                    <label for="mcp-days">{move_tr!("mcp-days")}</label>
                    <select id="mcp-days" on:change=move |ev| days.set(event_target_value(&ev).parse().unwrap_or(90))>
                        <option value="30" selected=move || days.get() == 30>"30"</option>
                        <option value="90" selected=move || days.get() == 90>"90"</option>
                        <option value="180" selected=move || days.get() == 180>"180"</option>
                        <option value="365" selected=move || days.get() == 365>"365"</option>
                    </select>
                    {field_error("lifetime_days", problems)}
                </div>
                <button type="submit" id="mcp-create-submit" class="vc-btn vc-btn--primary" disabled=move || !ready.get() || create.pending().get()>
                    {move_tr!("mcp-create-button")}
                </button>
            </form>
        </div>
    }
}

/// The value of the token just created, shown once with the command to plug it into Claude Code.
#[component]
fn CreatedToken(created: RwSignal<Option<CreatedMcpTokenDto>>) -> impl IntoView {
    view! {
        <Show when=move || created.with(Option::is_some)>
            <div class="vc-alert vc-alert--warning" id="mcp-token" role="status">
                <p>{move || tr!("mcp-token-once", {"name" => created.get().map(|t| t.token.name).unwrap_or_default()})}</p>
                <p><code id="mcp-token-value">{move || created.get().map(|t| t.value).unwrap_or_default()}</code></p>
                <p class="vc-muted">{move_tr!("mcp-token-command")}</p>
                <p><code id="mcp-token-command">{move || created.get().map(|t| t.command).unwrap_or_default()}</code></p>
                <button type="button" id="mcp-token-hide" class="vc-btn vc-btn--ghost vc-btn--sm" on:click=move |_| created.set(None)>
                    {move_tr!("mcp-token-hide")}
                </button>
            </div>
        </Show>
    }
}

#[component]
fn TokenTable(tokens: Vec<McpTokenDto>, version: RwSignal<u32>) -> impl IntoView {
    let ready = use_hydrated();
    let confirm = RwSignal::new(None::<i64>);
    let revoke = Action::new(|id: &i64| {
        let id = *id;
        async move { revoke_mcp_token(id).await }
    });
    Effect::new(move |_| {
        if revoke.value().get().is_some() {
            confirm.set(None);
            version.update(|v| *v += 1);
        }
    });
    if tokens.is_empty() {
        return view! { <p class="vc-muted" id="mcp-no-token">{move_tr!("mcp-no-token")}</p> }
            .into_any();
    }
    view! {
        <div class="vc-card">
            <table class="vc-table" id="mcp-tokens-table">
                <thead>
                    <tr>
                        <th>{move_tr!("mcp-col-name")}</th>
                        <th>{move_tr!("mcp-col-prefix")}</th>
                        <th>{move_tr!("mcp-col-scopes")}</th>
                        <th>{move_tr!("mcp-col-expires")}</th>
                        <th>{move_tr!("mcp-col-used")}</th>
                        <th>{move_tr!("mcp-col-state")}</th>
                        <th></th>
                    </tr>
                </thead>
                <tbody>
                    {tokens.into_iter().map(|t| {
                        let id = t.id;
                        let active = t.state == "active";
                        // Fermetures : les mots traduits suivent la langue choisie, sans recharger la liste.
                        let (scope_names, last_used, state) = (t.scopes.clone(), t.last_used_at.clone(), t.state.clone());
                        let scopes = move || scope_names.iter().map(|s| scope_text(s)).collect::<Vec<_>>().join(", ");
                        let used = move || if last_used.is_empty() { tr!("mcp-never-used") } else { last_used.clone() };
                        view! {
                            <tr data-id=id data-state=t.state.clone()>
                                <td>{t.name.clone()}</td>
                                <td><code>{t.prefix.clone()}</code></td>
                                <td>{scopes}</td>
                                <td class="vc-muted">{t.expires_at.clone()}</td>
                                <td class="vc-muted">{used}</td>
                                <td>{move || state_text(&state)}</td>
                                <td>
                                    {active.then(|| view! {
                                        <Show
                                            when=move || confirm.get() == Some(id)
                                            fallback=move || view! {
                                                <button type="button" class="vc-btn vc-btn--ghost vc-btn--sm mcp-revoke"
                                                    disabled=move || !ready.get()
                                                    on:click=move |_| confirm.set(Some(id))>
                                                    {move_tr!("mcp-revoke")}
                                                </button>
                                            }
                                        >
                                            <button type="button" class="vc-btn vc-btn--danger vc-btn--sm mcp-revoke-confirm"
                                                disabled=move || revoke.pending().get()
                                                on:click=move |_| { revoke.dispatch(id); }>
                                                {move_tr!("mcp-revoke-confirm")}
                                            </button>
                                            <button type="button" class="vc-btn vc-btn--ghost vc-btn--sm" on:click=move |_| confirm.set(None)>
                                                {move_tr!("crud-cancel")}
                                            </button>
                                        </Show>
                                    })}
                                </td>
                            </tr>
                        }
                    }).collect_view()}
                </tbody>
            </table>
        </div>
    }
    .into_any()
}

#[component]
fn AuditTable(version: RwSignal<u32>) -> impl IntoView {
    let kind = RwSignal::new(String::new());
    let lines = Resource::new(
        move || (version.get(), kind.get()),
        |(_, kind)| async move {
            list_mcp_audit(McpAuditFilter { kind, limit: 100 })
                .await
                .map_err(|_| ())
        },
    );
    view! {
        <div class="vc-card" id="mcp-audit">
            <h3>{move_tr!("mcp-audit-title")}</h3>
            <label>{move_tr!("mcp-audit-kind")}
                <select id="mcp-audit-kind" on:change=move |ev| kind.set(event_target_value(&ev))>
                    <option value="" selected=move || kind.get().is_empty()>{move_tr!("mcp-audit-all")}</option>
                    <option value="token_created" selected=move || kind.get() == "token_created">{move_tr!("mcp-kind-created")}</option>
                    <option value="token_revoked" selected=move || kind.get() == "token_revoked">{move_tr!("mcp-kind-revoked")}</option>
                    <option value="tool_call" selected=move || kind.get() == "tool_call">{move_tr!("mcp-kind-call")}</option>
                    <option value="denied" selected=move || kind.get() == "denied">{move_tr!("mcp-kind-denied")}</option>
                    <option value="agent" selected=move || kind.get() == "agent">{move_tr!("mcp-kind-agent")}</option>
                </select>
            </label>
            <Transition fallback=|| ()>
                {move || match lines.get() {
                    None => ().into_any(),
                    Some(Err(())) => view! { <div class="vc-alert vc-alert--error" role="alert">{move_tr!("crud-load-error")}</div> }.into_any(),
                    Some(Ok(rows)) if rows.is_empty() => view! { <p class="vc-muted" id="mcp-audit-empty">{move_tr!("mcp-audit-empty")}</p> }.into_any(),
                    Some(Ok(rows)) => view! {
                        <table class="vc-table" id="mcp-audit-table">
                            <thead>
                                <tr>
                                    <th>{move_tr!("mcp-audit-col-time")}</th>
                                    <th>{move_tr!("mcp-audit-col-actor")}</th>
                                    <th>{move_tr!("mcp-audit-col-kind")}</th>
                                    <th>{move_tr!("mcp-audit-col-tool")}</th>
                                    <th>{move_tr!("mcp-audit-col-outcome")}</th>
                                    <th>{move_tr!("mcp-audit-col-detail")}</th>
                                </tr>
                            </thead>
                            <tbody>
                                {rows.into_iter().map(|r| view! {
                                    <tr data-kind=r.kind.clone()>
                                        <td class="vc-muted">{r.at.clone()}</td>
                                        <td>{r.actor.clone()}</td>
                                        <td>{{ let kind = r.kind.clone(); move || kind_text(&kind) }}</td>
                                        <td>{r.tool.clone()}</td>
                                        <td>{r.outcome.clone()}</td>
                                        <td class="vc-muted" title=r.params.clone()>{r.detail.clone()}</td>
                                    </tr>
                                }).collect_view()}
                            </tbody>
                        </table>
                    }.into_any(),
                }}
            </Transition>
            <p class="vc-muted vc-hint">{move_tr!("mcp-audit-note")}</p>
        </div>
    }
}
