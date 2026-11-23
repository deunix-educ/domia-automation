// SPDX-License-Identifier: AGPL-3.0-or-later
//! Application log viewing: `/admin/logs` (administrators). The lines come from the server's in-memory buffer
//! (`dominet_server::logbuf`): this process, since its start, most recent first, with filters (level, module, text, period).

use std::time::Duration;

use leptos::prelude::*;
use leptos_fluent::{move_tr, tr};
use serde::{Deserialize, Serialize};

use crate::guard::RequireAdmin;

/// Filter entered on the page.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LogFilter {
    /// `error`, `warn`, `info`, `debug` or `trace`: the minimum severity level.
    pub level: String,
    pub target: String,
    pub text: String,
    /// Window in minutes (0: everything in memory).
    pub minutes: u32,
    pub limit: u32,
}

/// A displayed line.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LogLineDto {
    pub at: String,
    pub level: String,
    pub target: String,
    pub message: String,
}

/// Response to a viewing request.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LogPageDto {
    pub lines: Vec<LogLineDto>,
    pub matched: u32,
    pub total: u32,
    pub capacity: u32,
    pub targets: Vec<String>,
}

/// Application log lines (administrators), most recent first; `limit` is capped at 5000.
///
/// # Errors
/// Refused (403) for a non-administrator.
#[server(name = ListLogs, prefix = "/sf", endpoint = "logs/list", input = server_fn::codec::Json)]
pub async fn list_logs(filter: LogFilter) -> Result<LogPageDto, ServerFnError> {
    use dominet_server::logbuf::{self, LogQuery};

    use crate::server_fns::require_admin;

    let state = expect_context::<dominet_server::AppState>();
    require_admin(&state).await?;
    let limit = usize::try_from(filter.limit).unwrap_or(200).clamp(1, 5_000);
    let since_ms = (filter.minutes > 0)
        .then(|| logbuf::now_ms().saturating_sub(i64::from(filter.minutes) * 60_000));
    let page = logbuf::global().query(&LogQuery {
        max_rank: logbuf::rank_of_name(&filter.level),
        target: filter.target,
        text: filter.text,
        since_ms,
        limit,
    });
    let tz = state.config.timezone.clone();
    Ok(LogPageDto {
        lines: page
            .entries
            .iter()
            .map(|e| LogLineDto {
                at: logbuf::format_local_name(e.at_ms, &tz),
                level: e.level().to_string(),
                target: e.target.clone(),
                message: e.message.clone(),
            })
            .collect(),
        matched: u32::try_from(page.matched).unwrap_or(u32::MAX),
        total: u32::try_from(page.total).unwrap_or(u32::MAX),
        capacity: u32::try_from(logbuf::CAPACITY).unwrap_or(u32::MAX),
        targets: page.targets,
    })
}

#[component]
pub fn LogsPage() -> impl IntoView {
    view! {
        <RequireAdmin>
            <main class="vc-container">
                <Logs/>
            </main>
        </RequireAdmin>
    }
}

#[component]
fn Logs() -> impl IntoView {
    let level = RwSignal::new("info".to_string());
    let target = RwSignal::new(String::new());
    let text = RwSignal::new(String::new());
    let minutes = RwSignal::new(60u32);
    let limit = RwSignal::new(200u32);
    let auto = RwSignal::new(false);
    let tick = RwSignal::new(0u32);

    // Automatic refresh every 5 s (after hydration only: an `Effect` does not run on the server side).
    Effect::new(move |_| {
        if auto.get()
            && let Ok(handle) = set_interval_with_handle(
                move || tick.update(|t| *t = t.wrapping_add(1)),
                Duration::from_secs(5),
            )
        {
            on_cleanup(move || handle.clear());
        }
    });

    let data = Resource::new(
        move || {
            (
                level.get(),
                target.get(),
                text.get(),
                minutes.get(),
                limit.get(),
                tick.get(),
            )
        },
        |(level, target, text, minutes, limit, _)| async move {
            list_logs(LogFilter {
                level,
                target,
                text,
                minutes,
                limit,
            })
            .await
            .map_err(|_| ())
        },
    );

    view! {
        <div class="vc-page-header">
            <h2>{move_tr!("logs-title")}</h2>
            <div class="vc-header-actions">
                <button type="button" id="logs-refresh" class="vc-btn vc-btn--ghost" on:click=move |_| tick.update(|t| *t = t.wrapping_add(1))>
                    {move_tr!("logs-refresh")}
                </button>
            </div>
        </div>
        <section class="vc-card" id="logs-filters">
            <div class="vc-logs-filters">
                <label>{move_tr!("logs-level")}
                    <select id="logs-level" prop:value=move || level.get() on:change=move |ev| level.set(event_target_value(&ev))>
                        <option value="error">{move_tr!("logs-level-error")}</option>
                        <option value="warn">{move_tr!("logs-level-warn")}</option>
                        <option value="info">{move_tr!("logs-level-info")}</option>
                        <option value="debug">{move_tr!("logs-level-debug")}</option>
                    </select>
                </label>
                <label>{move_tr!("logs-module")}
                    <input type="search" id="logs-target" list="logs-targets" placeholder=move || tr!("logs-module-hint")
                        prop:value=move || target.get() on:input=move |ev| target.set(event_target_value(&ev))/>
                </label>
                <label>{move_tr!("logs-text")}
                    <input type="search" id="logs-text" placeholder=move || tr!("logs-text-hint")
                        prop:value=move || text.get() on:input=move |ev| text.set(event_target_value(&ev))/>
                </label>
                <label>{move_tr!("logs-period")}
                    <select id="logs-minutes" on:change=move |ev| minutes.set(event_target_value(&ev).parse().unwrap_or(0))>
                        <option value="15" selected=move || minutes.get() == 15>{move_tr!("logs-period-15")}</option>
                        <option value="60" selected=move || minutes.get() == 60>{move_tr!("logs-period-60")}</option>
                        <option value="360" selected=move || minutes.get() == 360>{move_tr!("logs-period-360")}</option>
                        <option value="1440" selected=move || minutes.get() == 1440>{move_tr!("logs-period-1440")}</option>
                        <option value="0" selected=move || minutes.get() == 0>{move_tr!("logs-period-all")}</option>
                    </select>
                </label>
                <label>{move_tr!("logs-limit")}
                    <select id="logs-limit" on:change=move |ev| limit.set(event_target_value(&ev).parse().unwrap_or(200))>
                        <option value="100" selected=move || limit.get() == 100>"100"</option>
                        <option value="200" selected=move || limit.get() == 200>"200"</option>
                        <option value="1000" selected=move || limit.get() == 1000>"1000"</option>
                        <option value="5000" selected=move || limit.get() == 5000>"5000"</option>
                    </select>
                </label>
                <label class="vc-logs-auto">
                    <input type="checkbox" id="logs-auto" prop:checked=move || auto.get() on:change=move |ev| auto.set(event_target_checked(&ev))/>
                    {move_tr!("logs-auto")}
                </label>
            </div>
            <Transition fallback=|| ()>
                {move || {
                    data.get().and_then(Result::ok).map(|page| {
                        view! { <datalist id="logs-targets">{page.targets.into_iter().map(|t| view! { <option value=t></option> }).collect_view()}</datalist> }
                    })
                }}
            </Transition>
        </section>
        <section class="vc-card vc-logs" id="logs-result">
            <Transition fallback=|| ()>
                {move || match data.get() {
                    None => ().into_any(),
                    Some(Err(())) => view! { <div class="vc-alert vc-alert--error" role="alert" id="logs-error">{move_tr!("crud-load-error")}</div> }.into_any(),
                    Some(Ok(page)) => {
                        let summary = tr!("logs-summary", {
                            "shown" => page.lines.len().to_string(),
                            "matched" => page.matched.to_string(),
                            "total" => page.total.to_string(),
                            "capacity" => page.capacity.to_string()
                        });
                        view! {
                            <p class="vc-muted" id="logs-summary">{summary}</p>
                            {if page.lines.is_empty() {
                                view! { <p id="logs-empty">{move_tr!("logs-empty")}</p> }.into_any()
                            } else {
                                view! {
                                    <table class="vc-table" id="logs-table">
                                        <thead><tr>
                                            <th>{move_tr!("logs-col-time")}</th>
                                            <th>{move_tr!("logs-col-level")}</th>
                                            <th>{move_tr!("logs-col-module")}</th>
                                            <th>{move_tr!("logs-col-message")}</th>
                                        </tr></thead>
                                        <tbody>
                                            {page.lines.into_iter().map(|l| {
                                                let class = format!("vc-log vc-log--{}", l.level.to_ascii_lowercase());
                                                view! {
                                                    <tr class=class>
                                                        <td class="vc-log-time">{l.at}</td>
                                                        <td class="vc-log-level">{l.level}</td>
                                                        <td class="vc-log-target">{l.target}</td>
                                                        <td class="vc-log-msg">{l.message}</td>
                                                    </tr>
                                                }
                                            }).collect_view()}
                                        </tbody>
                                    </table>
                                }.into_any()
                            }}
                            <p class="vc-muted vc-hint">{move_tr!("logs-note")}</p>
                        }.into_any()
                    }
                }}
            </Transition>
        </section>
    }
}
