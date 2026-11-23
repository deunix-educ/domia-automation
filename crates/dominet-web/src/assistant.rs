// SPDX-License-Identifier: AGPL-3.0-or-later
//! The local assistant: `/admin/assistant` (administrators; R12 lot A2, ADR 0060).
//!
//! The page asks the server for the new events of the conversation every second (no push): the answer of the model, the
//! action cards, what a confirmed card did. **Nothing is changed unless the administrator clicks "Confirm" on a card**: the
//! model only ever proposes (`ROADMAP-agent.md` section 4). The conversation lives in the server's memory.

use std::time::Duration;

use leptos::prelude::*;
use leptos_fluent::{move_tr, tr};
use serde::{Deserialize, Serialize};

use crate::guard::RequireAdmin;
use crate::layout::use_hydrated;

/// One line of the conversation, as the page draws it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AssistantEventDto {
    /// `user`, `status`, `reply`, `card`, `done`, `undone`, `dropped`, `failed`, `idle`.
    pub kind: String,
    /// Card number for `card`, `done` and `dropped`.
    pub id: u64,
    pub text: String,
    pub tool: String,
    /// Compact JSON of the arguments (cards only).
    pub arguments: String,
    /// What changes, one sentence per field (configuration cards).
    pub lines: Vec<String>,
    /// The YAML of an automation before and after (`+ `, `- `, or two spaces first), automations only.
    pub diff: Vec<String>,
}

/// What a poll brings back.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AssistantPollDto {
    pub enabled: bool,
    pub events: Vec<AssistantEventDto>,
    pub next: usize,
    pub busy: bool,
    pub reset: bool,
}

/// A change made through the assistant, as the page lists it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AssistantChangeDto {
    pub id: i64,
    pub at: String,
    pub label: String,
    /// What was asked, in clear.
    pub request: String,
    pub undoable: bool,
    /// Why not, when it cannot be undone.
    pub reason: String,
}

/// The changes made through the assistant that stand (newest first).
///
/// # Errors
/// Refused (403) for a non-administrator.
#[server(name = AssistantChanges, prefix = "/sf", endpoint = "agent/changes", input = server_fn::codec::Json)]
pub async fn assistant_changes() -> Result<Vec<AssistantChangeDto>, ServerFnError> {
    use crate::server_fns::require_admin;

    let state = expect_context::<dominet_server::AppState>();
    require_admin(&state).await?;
    Ok(dominet_agent::hub()
        .changes(&state)
        .await
        .into_iter()
        .take(30)
        .map(|c| AssistantChangeDto {
            id: c.id,
            at: dominet_server::timefmt::text(c.at),
            label: c.label,
            request: c.request,
            undoable: c.plan.is_some(),
            reason: c.reason,
        })
        .collect())
}

/// Takes a change back (the administrator clicked "Undo"). Returns the reason in words if refused.
///
/// # Errors
/// Refused (403) for a non-administrator.
#[server(name = AssistantUndo, prefix = "/sf", endpoint = "agent/undo", input = server_fn::codec::Json)]
pub async fn assistant_undo(id: i64) -> Result<Option<String>, ServerFnError> {
    use crate::server_fns::require_admin;

    let state = expect_context::<dominet_server::AppState>();
    let admin = require_admin(&state).await?;
    Ok(Box::pin(dominet_agent::hub().undo(&state, &admin, id))
        .await
        .err()
        .map(|e| e.to_string()))
}

/// The new events of the conversation, from `since`.
///
/// # Errors
/// Refused (403) for a non-administrator.
#[server(name = AssistantPoll, prefix = "/sf", endpoint = "agent/poll", input = server_fn::codec::Json)]
pub async fn assistant_poll(since: usize) -> Result<AssistantPollDto, ServerFnError> {
    use dominet_agent::{AgentHub, Event};

    use crate::server_fns::require_admin;

    let state = expect_context::<dominet_server::AppState>();
    let admin = require_admin(&state).await?;
    let enabled = AgentHub::enabled(&state).await;
    let polled = dominet_agent::hub().poll(&admin, since);
    let dto =
        |kind: &str, id: u64, text: String, tool: String, arguments: String| AssistantEventDto {
            kind: kind.to_string(),
            id,
            text,
            tool,
            arguments,
            lines: Vec::new(),
            diff: Vec::new(),
        };
    let events = polled
        .events
        .into_iter()
        .map(|e| match e {
            Event::User(t) => dto("user", 0, t, String::new(), String::new()),
            Event::Status(t) => dto("status", 0, t, String::new(), String::new()),
            Event::Reply(t) => dto("reply", 0, t, String::new(), String::new()),
            Event::Card {
                id,
                tool,
                arguments,
                summary,
                lines,
                diff,
            } => AssistantEventDto {
                lines,
                diff,
                ..dto("card", id, summary, tool, arguments)
            },
            Event::Done { id, text } => dto("done", id, text, String::new(), String::new()),
            Event::Dropped { id, reason } => {
                dto("dropped", id, reason, String::new(), String::new())
            }
            Event::Undone(t) => dto("undone", 0, t, String::new(), String::new()),
            Event::Failed(t) => dto("failed", 0, t, String::new(), String::new()),
            Event::Idle => dto("idle", 0, String::new(), String::new(), String::new()),
        })
        .collect();
    Ok(AssistantPollDto {
        enabled,
        events,
        next: polled.next,
        busy: polled.busy,
        reset: polled.reset,
    })
}

/// Sends a message. `regime` is `daily` or `config`. Returns the reason in words if the request is refused, `None` if it started.
///
/// # Errors
/// Refused (403) for a non-administrator.
#[server(name = AssistantSend, prefix = "/sf", endpoint = "agent/send", input = server_fn::codec::Json)]
pub async fn assistant_send(text: String, regime: String) -> Result<Option<String>, ServerFnError> {
    use dominet_agent::Regime;

    use crate::server_fns::require_admin;

    let state = expect_context::<dominet_server::AppState>();
    let admin = require_admin(&state).await?;
    let regime = if regime == "config" {
        Regime::Config
    } else {
        Regime::Daily
    };
    Ok(dominet_agent::hub()
        .send(&state, &admin, &text, regime)
        .await
        .err()
        .map(|e| e.to_string()))
}

/// The administrator clicked "Confirm" on a card: the only way anything is done. Returns the reason in words if refused.
///
/// # Errors
/// Refused (403) for a non-administrator.
#[server(name = AssistantConfirm, prefix = "/sf", endpoint = "agent/confirm", input = server_fn::codec::Json)]
pub async fn assistant_confirm(id: u64) -> Result<Option<String>, ServerFnError> {
    use crate::server_fns::require_admin;

    let state = expect_context::<dominet_server::AppState>();
    let admin = require_admin(&state).await?;
    Ok(Box::pin(dominet_agent::hub().confirm(&state, &admin, id))
        .await
        .err()
        .map(|e| e.to_string()))
}

/// The administrator turned a card down.
///
/// # Errors
/// Refused (403) for a non-administrator.
#[server(name = AssistantCancel, prefix = "/sf", endpoint = "agent/cancel", input = server_fn::codec::Json)]
pub async fn assistant_cancel(id: u64) -> Result<Option<String>, ServerFnError> {
    use crate::server_fns::require_admin;

    let state = expect_context::<dominet_server::AppState>();
    let admin = require_admin(&state).await?;
    Ok(dominet_agent::hub()
        .cancel(&state, &admin, id)
        .await
        .err()
        .map(|e| e.to_string()))
}

/// Starts a new conversation.
///
/// # Errors
/// Refused (403) for a non-administrator.
#[server(name = AssistantReset, prefix = "/sf", endpoint = "agent/reset", input = server_fn::codec::Json)]
pub async fn assistant_reset() -> Result<(), ServerFnError> {
    use crate::server_fns::require_admin;

    let state = expect_context::<dominet_server::AppState>();
    let admin = require_admin(&state).await?;
    dominet_agent::hub().reset(&admin);
    Ok(())
}

/// Tests the model server set in `AGENT_LLM_URL`: the models it offers, or why it does not answer.
///
/// # Errors
/// Refused (403) for a non-administrator.
#[server(name = AssistantTest, prefix = "/sf", endpoint = "agent/test", input = server_fn::codec::Json)]
pub async fn assistant_test() -> Result<String, ServerFnError> {
    use crate::server_fns::require_admin;

    let state = expect_context::<dominet_server::AppState>();
    require_admin(&state).await?;
    let url = state
        .settings
        .get_string(dominet_agent::service::SETTING_URL)
        .await
        .unwrap_or_default();
    Ok(match dominet_agent::test_connection(&url).await {
        Ok(models) if models.is_empty() => "ok".to_string(),
        Ok(models) => format!("ok : {}", models.join(", ")),
        Err(e) => format!("failure: {e}"),
    })
}

#[component]
pub fn AssistantPage() -> impl IntoView {
    view! {
        <RequireAdmin>
            <main class="vc-container">
                <Assistant/>
            </main>
        </RequireAdmin>
    }
}

/// Readable form of a card's arguments: `cle = valeur`, one per line.
#[must_use]
pub fn arguments_lines(arguments: &str) -> Vec<String> {
    match serde_json::from_str::<serde_json::Value>(arguments) {
        Ok(serde_json::Value::Object(map)) => map
            .iter()
            .map(|(k, v)| match v {
                serde_json::Value::String(s) => format!("{k} = {s}"),
                other => format!("{k} = {other}"),
            })
            .collect(),
        _ => vec![arguments.to_string()],
    }
}

#[component]
fn Assistant() -> impl IntoView {
    let ready = use_hydrated();
    let since = RwSignal::new(0usize);
    let events = RwSignal::new(Vec::<AssistantEventDto>::new());
    let busy = RwSignal::new(false);
    let enabled = RwSignal::new(true);
    let regime = RwSignal::new("daily".to_string());
    let draft = RwSignal::new(String::new());
    let notice = RwSignal::new(String::new());
    let test_result = RwSignal::new(String::new());
    let tick = RwSignal::new(0u32);
    // Bumped when something was done or undone, so that the list of changes is read again.
    let version = RwSignal::new(0u32);

    // One poll per second once the page is hydrated (an `Effect` does not run on the server).
    Effect::new(move |_| {
        if let Ok(handle) = set_interval_with_handle(
            move || tick.update(|t| *t = t.wrapping_add(1)),
            Duration::from_secs(1),
        ) {
            on_cleanup(move || handle.clear());
        }
    });
    let poll = Action::new(|since: &usize| {
        let since = *since;
        async move { assistant_poll(since).await }
    });
    Effect::new(move |_| {
        tick.track();
        if !poll.pending().get_untracked() {
            poll.dispatch(since.get_untracked());
        }
    });
    Effect::new(move |_| {
        if let Some(Ok(p)) = poll.value().get() {
            enabled.set(p.enabled);
            busy.set(p.busy);
            if p.reset {
                events.set(Vec::new());
            }
            since.set(p.next);
            if p.events
                .iter()
                .any(|e| e.kind == "done" || e.kind == "undone")
            {
                version.update(|v| *v += 1);
            }
            if !p.events.is_empty() {
                events.update(|all| all.extend(p.events));
            }
        }
    });

    let send = Action::new(|(text, regime): &(String, String)| {
        let (text, regime) = (text.clone(), regime.clone());
        async move { assistant_send(text, regime).await }
    });
    Effect::new(move |_| {
        if let Some(result) = send.value().get() {
            match result {
                Ok(None) => {
                    notice.set(String::new());
                    draft.set(String::new());
                    busy.set(true);
                    tick.update(|t| *t = t.wrapping_add(1));
                }
                Ok(Some(reason)) => notice.set(reason),
                Err(_) => notice.set(tr!("assistant-error")),
            }
        }
    });
    let submit = move |ev: leptos::ev::SubmitEvent| {
        ev.prevent_default();
        let text = draft.get_untracked();
        if !text.trim().is_empty() {
            send.dispatch((text, regime.get_untracked()));
        }
    };
    let reset = Action::new(|(): &()| async move { assistant_reset().await });
    Effect::new(move |_| {
        if reset.value().get().is_some() {
            events.set(Vec::new());
            tick.update(|t| *t = t.wrapping_add(1));
        }
    });
    let test = Action::new(|(): &()| async move { assistant_test().await });
    Effect::new(move |_| {
        if let Some(r) = test.value().get() {
            test_result.set(r.unwrap_or_else(|_| tr!("assistant-error")));
        }
    });

    let rows = move || events.get().into_iter().enumerate().collect::<Vec<_>>();
    let row_key = |row: &(usize, AssistantEventDto)| (row.0, row.1.kind.clone(), row.1.id);
    view! {
        <div class="vc-page-header">
            <h2>{move_tr!("assistant-title")}</h2>
        </div>
        <p class="vc-muted">{move_tr!("assistant-intro")}</p>
        <Show when=move || !enabled.get()>
            <div class="vc-alert vc-alert--warning" id="assistant-off" role="status">
                {move_tr!("assistant-off")}" "
                <a href="/admin/settings">{move_tr!("assistant-settings-link")}</a>
            </div>
        </Show>
        <div class="vc-card">
            <ul class="vc-chat" id="assistant-log" aria-live="polite">
                <For each=rows key=row_key children=move |(_, e)| view! { <ChatLine event=e events=events notice=notice/> }/>
            </ul>
            <Show when=move || busy.get()>
                <p class="vc-muted" id="assistant-busy" role="status">{move_tr!("assistant-busy")}</p>
            </Show>
            <Show when=move || !notice.get().is_empty()>
                <div class="vc-alert vc-alert--error" id="assistant-notice" role="alert">{move || notice.get()}</div>
            </Show>
            <form on:submit=submit id="assistant-form">
                <div class="vc-form-group">
                    <label for="assistant-input">{move_tr!("assistant-input")}</label>
                    <textarea
                        id="assistant-input"
                        rows="2"
                        maxlength="2000"
                        placeholder=move || tr!("assistant-placeholder")
                        prop:value=move || draft.get()
                        on:input=move |ev| draft.set(event_target_value(&ev))
                    ></textarea>
                </div>
                <div class="vc-form-group">
                    <label for="assistant-regime">{move_tr!("assistant-regime")}</label>
                    <select id="assistant-regime" on:change=move |ev| regime.set(event_target_value(&ev))>
                        <option value="daily" selected=move || regime.get() == "daily">{move_tr!("assistant-regime-daily")}</option>
                        <option value="config" selected=move || regime.get() == "config">{move_tr!("assistant-regime-config")}</option>
                    </select>
                </div>
                <button
                    type="submit"
                    id="assistant-send"
                    class="vc-btn vc-btn--primary"
                    disabled=move || !ready.get() || busy.get() || send.pending().get() || !enabled.get()
                >
                    {move_tr!("assistant-send")}
                </button>
                " "
                <button
                    type="button"
                    id="assistant-reset"
                    class="vc-btn vc-btn--ghost"
                    disabled=move || !ready.get() || busy.get()
                    on:click=move |_| { reset.dispatch(()); }
                >
                    {move_tr!("assistant-reset")}
                </button>
            </form>
        </div>
        <ChangesList version=version notice=notice/>
        <div class="vc-card">
            <h3>{move_tr!("assistant-test-title")}</h3>
            <button
                type="button"
                id="assistant-test"
                class="vc-btn vc-btn--ghost"
                disabled=move || !ready.get() || test.pending().get()
                on:click=move |_| { test.dispatch(()); }
            >
                {move_tr!("assistant-test")}
            </button>
            <p id="assistant-test-result" class="vc-muted">{move || test_result.get()}</p>
        </div>
    }
}

/// The changes made through the assistant, each with its "Undo" (two clicks: the first only asks).
#[component]
fn ChangesList(version: RwSignal<u32>, notice: RwSignal<String>) -> impl IntoView {
    let ready = use_hydrated();
    let asking = RwSignal::new(None::<i64>);
    let data = Resource::new(
        move || version.get(),
        |_| async { assistant_changes().await.map_err(|_| ()) },
    );
    let undo = Action::new(|id: &i64| {
        let id = *id;
        async move { Box::pin(assistant_undo(id)).await }
    });
    Effect::new(move |_| {
        if let Some(r) = undo.value().get() {
            asking.set(None);
            match r {
                Ok(Some(reason)) => notice.set(reason),
                Ok(None) => notice.set(String::new()),
                Err(_) => notice.set(tr!("assistant-error")),
            }
            version.update(|v| *v += 1);
        }
    });
    view! {
        <div class="vc-card" id="assistant-changes">
            <h3>{move_tr!("assistant-changes-title")}</h3>
            <Transition fallback=|| ()>
                {move || match data.get() {
                    None | Some(Err(())) => ().into_any(),
                    Some(Ok(list)) if list.is_empty() => view! { <p class="vc-muted" id="assistant-no-change">{move_tr!("assistant-no-change")}</p> }.into_any(),
                    Some(Ok(list)) => view! {
                        <ul class="vc-chat__changes">
                            {list.into_iter().map(|c| {
                                let id = c.id;
                                view! {
                                    <li id=format!("assistant-change-{id}")>
                                        <strong>{c.label}</strong>
                                        <span class="vc-muted">" · "{c.at}</span>
                                        <br/>
                                        <span class="vc-muted">{move_tr!("assistant-asked")}" : "{c.request}</span>
                                        <br/>
                                        {if c.undoable {
                                            view! {
                                                <Show
                                                    when=move || asking.get() == Some(id)
                                                    fallback=move || view! {
                                                        <button type="button" class="vc-btn vc-btn--ghost vc-btn--sm assistant-undo" disabled=move || !ready.get() on:click=move |_| asking.set(Some(id))>
                                                            {move_tr!("assistant-undo")}
                                                        </button>
                                                    }
                                                >
                                                    <button type="button" class="vc-btn vc-btn--primary vc-btn--sm assistant-undo-confirm" disabled=move || undo.pending().get() on:click=move |_| { undo.dispatch(id); }>
                                                        {move_tr!("assistant-undo-confirm")}
                                                    </button>
                                                    " "
                                                    <button type="button" class="vc-btn vc-btn--ghost vc-btn--sm assistant-undo-no" on:click=move |_| asking.set(None)>
                                                        {move_tr!("assistant-undo-no")}
                                                    </button>
                                                </Show>
                                            }.into_any()
                                        } else {
                                            view! { <span class="vc-muted">{move_tr!("assistant-not-undoable")}" : "{c.reason}</span> }.into_any()
                                        }}
                                    </li>
                                }
                            }).collect_view()}
                        </ul>
                    }.into_any(),
                }}
            </Transition>
        </div>
    }
}

/// One line of the conversation; a card carries its two buttons.
#[component]
fn ChatLine(
    event: AssistantEventDto,
    events: RwSignal<Vec<AssistantEventDto>>,
    notice: RwSignal<String>,
) -> impl IntoView {
    let ready = use_hydrated();
    let kind = event.kind.clone();
    match kind.as_str() {
        "user" => view! { <li class="vc-chat__user">{event.text}</li> }.into_any(),
        "reply" => view! { <li class="vc-chat__reply">{event.text}</li> }.into_any(),
        "undone" => view! {
            <li class="vc-chat__done">{move_tr!("assistant-undone")}" : "{event.text}</li>
        }
        .into_any(),
        "done" => view! {
            <li class="vc-chat__done">{move_tr!("assistant-done")}" : "{event.text}</li>
        }
        .into_any(),
        "dropped" => view! { <li class="vc-muted">{move_tr!("assistant-dropped")}</li> }.into_any(),
        "failed" => view! { <li class="vc-chat__failed" role="alert">{event.text}</li> }.into_any(),
        "card" => {
            let id = event.id;
            // A card that was handled (confirmed, cancelled, expired) later in the log has no more buttons.
            let handled = Memo::new(move |_| {
                events.with(|all| {
                    all.iter()
                        .any(|e| e.id == id && (e.kind == "done" || e.kind == "dropped"))
                })
            });
            let confirm =
                Action::new(move |(): &()| async move { Box::pin(assistant_confirm(id)).await });
            let cancel = Action::new(move |(): &()| async move { assistant_cancel(id).await });
            Effect::new(move |_| {
                for r in [confirm.value().get(), cancel.value().get()]
                    .into_iter()
                    .flatten()
                {
                    match r {
                        Ok(Some(reason)) => notice.set(reason),
                        Ok(None) => notice.set(String::new()),
                        Err(_) => notice.set(tr!("assistant-error")),
                    }
                }
            });
            let lines = arguments_lines(&event.arguments);
            view! {
                <li class="vc-chat__card" id=format!("assistant-card-{id}")>
                    <strong>{move_tr!("assistant-card-title")}</strong>
                    {(!event.lines.is_empty()).then(|| view! {
                        <ul class="vc-chat__changes-lines">
                            {event.lines.iter().map(|l| view! { <li>{l.clone()}</li> }).collect_view()}
                        </ul>
                    })}
                    <details class="vc-chat__tech" open=event.lines.is_empty()>
                        <summary>{move_tr!("assistant-details")}</summary>
                        <code>{event.tool}</code>
                        <ul class="vc-chat__args">
                            {lines.into_iter().map(|l| view! { <li><code>{l}</code></li> }).collect_view()}
                        </ul>
                        <p class="vc-muted">{event.text}</p>
                    </details>
                    {(!event.diff.is_empty()).then(|| view! {
                        <details class="vc-chat__yaml">
                            <summary>{move_tr!("assistant-yaml")}</summary>
                            <pre>
                                {event.diff.iter().map(|l| {
                                    let class = match l.chars().next() { Some('+') => "vc-diff-add", Some('-') => "vc-diff-del", _ => "vc-diff-same" };
                                    view! { <span class=class>{l.clone()}"\n"</span> }
                                }).collect_view()}
                            </pre>
                        </details>
                    })}
                    <Show when=move || !handled.get()>
                        <button
                            type="button"
                            class="vc-btn vc-btn--primary assistant-confirm"
                            disabled=move || !ready.get() || confirm.pending().get()
                            on:click=move |_| { confirm.dispatch(()); }
                        >
                            {move_tr!("assistant-confirm")}
                        </button>
                        " "
                        <button
                            type="button"
                            class="vc-btn vc-btn--ghost assistant-cancel"
                            disabled=move || !ready.get() || cancel.pending().get()
                            on:click=move |_| { cancel.dispatch(()); }
                        >
                            {move_tr!("assistant-cancel")}
                        </button>
                    </Show>
                </li>
            }
            .into_any()
        }
        _ => ().into_any(),
    }
}

#[cfg(test)]
mod tests {
    use super::arguments_lines;

    #[test]
    fn arguments_are_shown_one_per_line() {
        let lines = arguments_lines(r#"{"device":"volet salon","command":"close"}"#);
        assert_eq!(lines.len(), 2);
        assert!(lines.iter().any(|l| l == "device = volet salon"));
        assert!(lines.iter().any(|l| l == "command = close"));
    }

    #[test]
    fn what_is_not_an_object_is_kept_as_is() {
        assert_eq!(
            arguments_lines("pas du json"),
            vec!["pas du json".to_string()]
        );
    }
}
