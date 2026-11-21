// SPDX-License-Identifier: AGPL-3.0-or-later
//! Alertes (R4 lot 6, ADR 0035) : la page `/alerts` (tout compte connecte : consulter, acquitter une alerte, tout acquitter) et la
//! pastille de la barre laterale. L'administration des regles est dans `alert_rules_admin`.

use dominet_proto::{AckOutcome, AlertsPageDto, Topic};
use leptos::prelude::*;
use leptos_fluent::{move_tr, tr};
use leptos_router::components::A;
use leptos_router::hooks::use_query_map;

use crate::guard::RequireAuth;
use crate::layout::use_hydrated;
use crate::realtime::Realtime;

/// Filtres admis de la page, dans l'ordre d'affichage.
const STATUSES: [&str; 4] = ["firing", "resolved", "acknowledged", "all"];

/// Ce que la page sait de Prometheus et de Grafana (dernier instantane lu en fond).
#[cfg(feature = "ssr")]
fn platform_dto(
    external: &dominet_server::external_alerts::ExternalAlerts,
) -> dominet_proto::PlatformAlertsDto {
    use dominet_server::external_alerts::SourceState;

    let snapshot = external.snapshot();
    let code = |s: &SourceState| {
        match s {
            SourceState::NotConfigured => "off",
            SourceState::Unreachable => "down",
            SourceState::Ok(_) => "ok",
        }
        .to_string()
    };
    dominet_proto::PlatformAlertsDto {
        prometheus: code(&snapshot.prometheus),
        grafana: code(&snapshot.grafana),
        alerts: snapshot
            .alerts()
            .into_iter()
            .map(|a| dominet_proto::PlatformAlertDto {
                source: a.source.name().to_string(),
                name: a.name,
                severity: a.severity,
                summary: a.summary,
                description: a.description,
                since: a.since.unwrap_or_default(),
            })
            .collect(),
    }
}

/// Les alertes (200 au plus, les plus recentes d'abord) filtrees par statut (`firing` par defaut ; tout autre texte = `firing`),
/// avec le nombre d'alertes en cours. Tout compte connecte.
///
/// # Errors
/// Refus (401) sans session ; erreur interne si la base echoue.
#[server(name = AlertsPage, prefix = "/sf", endpoint = "alerts/page")]
pub async fn alerts_page(status: String) -> Result<AlertsPageDto, ServerFnError> {
    use std::collections::HashMap;

    use dominet_domain::alert::AlertStatus;
    use dominet_domain::repo::{AlertRepo, AlertRuleRepo, DeviceFilter, DeviceRepo, UserRepo};

    use crate::server_fns::{internal, require_user};

    let state = expect_context::<dominet_server::AppState>();
    require_user(&state).await?;
    let status = if STATUSES.contains(&status.as_str()) {
        status
    } else {
        "firing".to_string()
    };
    let filter = AlertStatus::parse(&status);
    let alerts = state
        .store
        .alerts()
        .list(filter, 200)
        .await
        .map_err(|e| internal("alert list", &e))?;
    let firing = state
        .store
        .alerts()
        .count_firing()
        .await
        .map_err(|e| internal("current alerts", &e))?;
    let rules: HashMap<i64, String> = state
        .store
        .alert_rules()
        .list()
        .await
        .map_err(|e| internal("alert rules", &e))?
        .into_iter()
        .map(|r| (r.id.0, r.rule.name))
        .collect();
    let devices: HashMap<i64, String> = state
        .store
        .devices()
        .list(&DeviceFilter::default())
        .await
        .map_err(|e| internal("device list", &e))?
        .into_iter()
        .map(|d| (d.id.0, d.friendly_name))
        .collect();
    let users: HashMap<i64, String> = state
        .store
        .users()
        .list()
        .await
        .map_err(|e| internal("account list", &e))?
        .into_iter()
        .map(|u| (u.id.0, u.username))
        .collect();
    let time = dominet_server::timefmt::rfc3339;
    let platform = platform_dto(&state.external_alerts);
    Ok(AlertsPageDto {
        status,
        platform,
        firing: u32::try_from(firing).unwrap_or(u32::MAX),
        alerts: alerts
            .into_iter()
            .map(|a| dominet_proto::AlertDto {
                id: a.id.0,
                rule: a
                    .rule
                    .and_then(|r| rules.get(&r.0).cloned())
                    .unwrap_or_default(),
                device_id: a.device.0,
                device: devices.get(&a.device.0).cloned().unwrap_or_default(),
                severity: a.severity.as_str().to_string(),
                status: a.status.as_str().to_string(),
                value: a.value_triggered,
                fired_at: time(a.fired_at),
                resolved_at: a.resolved_at.map(time).unwrap_or_default(),
                acknowledged_at: a.acknowledged_at.map(time).unwrap_or_default(),
                acknowledged_by: a
                    .acknowledged_by
                    .and_then(|u| users.get(&u.0).cloned())
                    .unwrap_or_default(),
            })
            .collect(),
    })
}

/// Acquitte une alerte en cours (tout compte connecte, comme en reference).
///
/// # Errors
/// Refus (401) sans session ; erreur interne si la base echoue.
#[server(name = AcknowledgeAlert, prefix = "/sf", endpoint = "alerts/acknowledge")]
pub async fn acknowledge_alert(id: i64) -> Result<AckOutcome, ServerFnError> {
    use dominet_domain::alert::AlertId;
    use dominet_domain::repo::AlertRepo;

    use crate::server_fns::{internal, require_user};

    let state = expect_context::<dominet_server::AppState>();
    let user = require_user(&state).await?;
    let done = state
        .store
        .alerts()
        .acknowledge(AlertId(id), user.id, chrono::Utc::now())
        .await
        .map_err(|e| internal("acknowledgement", &e))?;
    if done.is_none() {
        return Ok(AckOutcome::NotFiring);
    }
    tracing::info!(user_id = user.id.0, alert_id = id, "alert acknowledged");
    dominet_server::alerts::publish_count(&state.hub, &state.store, &state.external_alerts).await;
    Ok(AckOutcome::Acknowledged)
}

/// Acquitte toutes les alertes en cours ; renvoie leur nombre.
///
/// # Errors
/// Refus (401) sans session ; erreur interne si la base echoue.
#[server(name = AcknowledgeAllAlerts, prefix = "/sf", endpoint = "alerts/acknowledge-all")]
pub async fn acknowledge_all_alerts() -> Result<u32, ServerFnError> {
    use dominet_domain::repo::AlertRepo;

    use crate::server_fns::{internal, require_user};

    let state = expect_context::<dominet_server::AppState>();
    let user = require_user(&state).await?;
    let n = state
        .store
        .alerts()
        .acknowledge_all(user.id, chrono::Utc::now())
        .await
        .map_err(|e| internal("acknowledgement", &e))?;
    tracing::info!(user_id = user.id.0, count = n, "acknowledged alerts");
    dominet_server::alerts::publish_count(&state.hub, &state.store, &state.external_alerts).await;
    Ok(u32::try_from(n).unwrap_or(u32::MAX))
}

#[component]
pub fn AlertsPage() -> impl IntoView {
    view! {
        <RequireAuth>
            <main class="vc-container">
                <AlertsContent/>
            </main>
        </RequireAuth>
    }
}

/// La section « Plateforme » : alertes de Prometheus et de Grafana, en lecture seule. Rien si aucune des deux sources n'est
/// surveillee ; une source injoignable est dite, jamais confondue avec « aucune alerte ».
fn platform_section(platform: &dominet_proto::PlatformAlertsDto) -> Option<AnyView> {
    if platform.prometheus == "off" && platform.grafana == "off" {
        return None;
    }
    let down: Vec<AnyView> = [
        ("Prometheus", &platform.prometheus),
        ("Grafana", &platform.grafana),
    ]
    .into_iter()
    .filter(|(_, state)| state.as_str() == "down")
    .map(|(name, _)| {
        let id = format!("platform-down-{}", name.to_lowercase());
        view! {
            <div class="vc-alert vc-alert--warning" role="status" id=id>
                {move_tr!("platform-down", {"source" => name})}
            </div>
        }
        .into_any()
    })
    .collect();
    let alerts = platform.alerts.clone();
    let quiet = alerts.is_empty() && down.is_empty();
    Some(
        view! {
            <section class="vc-card" id="platform-alerts">
                <h3>{move_tr!("platform-title")}</h3>
                <p class="vc-muted">{move_tr!("platform-readonly")}</p>
                {down}
                {quiet.then(|| view! { <p class="vc-muted" id="platform-empty">{move_tr!("platform-empty")}</p> })}
                {(!alerts.is_empty())
                    .then(|| {
                        view! {
                            <table class="vc-table" id="platform-table">
                                <thead>
                                    <tr>
                                        <th>{move_tr!("alerts-col-severity")}</th>
                                        <th>{move_tr!("platform-col-source")}</th>
                                        <th>{move_tr!("platform-col-name")}</th>
                                        <th>{move_tr!("platform-col-summary")}</th>
                                        <th>{move_tr!("platform-col-since")}</th>
                                    </tr>
                                </thead>
                                <tbody>
                                    {alerts
                                        .into_iter()
                                        .map(|a| {
                                            view! {
                                                <tr data-source=a.source.clone() data-severity=a.severity.clone()>
                                                    <td>
                                                        <span
                                                            class="vc-badge"
                                                            class:vc-badge--danger=a.severity == "critical"
                                                            class:vc-badge--warning=a.severity == "warning"
                                                        >
                                                            {severity_label(&a.severity)}
                                                        </span>
                                                    </td>
                                                    <td>{if a.source == "grafana" { "Grafana" } else { "Prometheus" }}</td>
                                                    <td>{a.name.clone()}</td>
                                                    <td title=a.description.clone()>{a.summary.clone()}</td>
                                                    <td>{shown(&a.since)}</td>
                                                </tr>
                                            }
                                        })
                                        .collect_view()}
                                </tbody>
                            </table>
                        }
                    })}
            </section>
        }
        .into_any(),
    )
}

fn status_label(status: &str) -> String {
    match status {
        "firing" => tr!("alerts-status-firing"),
        "resolved" => tr!("alerts-status-resolved"),
        "acknowledged" => tr!("alerts-status-acknowledged"),
        _ => tr!("alerts-status-all"),
    }
}

fn severity_label(severity: &str) -> String {
    match severity {
        "info" => tr!("alerts-severity-info"),
        "warning" => tr!("alerts-severity-warning"),
        _ => tr!("alerts-severity-critical"),
    }
}

use crate::android::shown;

#[component]
fn AlertsContent() -> impl IntoView {
    let rt = expect_context::<Realtime>();
    Effect::new(move |_| rt.subscribe(Topic::Alerts));
    on_cleanup(move || rt.unsubscribe(&Topic::Alerts));
    let ready = use_hydrated();
    let query = use_query_map();
    let status = move || query.with(|q| q.get("status").unwrap_or_else(|| "firing".to_string()));
    let version = RwSignal::new(0u32);
    // Le nombre d'alertes en cours change (declenchement, resolution, acquittement ailleurs) : la liste se recharge.
    let list = Resource::new(
        move || (status(), version.get(), rt.alerts.get()),
        |(s, _, _)| async move { alerts_page(s).await.map_err(|_| ()) },
    );
    let ack = Action::new(|id: &i64| {
        let id = *id;
        async move { acknowledge_alert(id).await }
    });
    let ack_all = Action::new(|(): &()| async { acknowledge_all_alerts().await });
    Effect::new(move |_| {
        if ack.version().get() + ack_all.version().get() > 0 {
            version.update(|v| *v += 1);
        }
    });
    view! {
        <div class="vc-page-header">
            <h2>{move_tr!("alerts-title")}</h2>
            <div class="vc-header-actions">
                <button
                    type="button"
                    id="alerts-ack-all"
                    class="vc-btn vc-btn--primary"
                    disabled=move || !ready.get() || ack_all.pending().get()
                    on:click=move |_| {
                        ack_all.dispatch(());
                    }
                >
                    {move_tr!("alerts-ack-all")}
                </button>
            </div>
        </div>
        <nav class="vc-tabs" id="alerts-filters">
            {STATUSES
                .iter()
                .map(|s| {
                    let s = *s;
                    view! {
                        <A
                            href=format!("/alerts?status={s}")
                            attr:class="vc-btn vc-btn--ghost vc-btn--sm"
                            attr:id=format!("alerts-filter-{s}")
                            attr:aria-current=move || (status() == s).then_some("page")
                        >
                            {move || status_label(s)}
                        </A>
                    }
                })
                .collect_view()}
        </nav>
        <Transition fallback=|| ()>
            {move || match list.get() {
                None => ().into_any(),
                Some(Err(())) => {
                    view! { <div class="vc-alert vc-alert--error" role="alert" id="alerts-load-error">{move_tr!("crud-load-error")}</div> }
                        .into_any()
                }
                Some(Ok(page)) if page.alerts.is_empty() => {
                    let platform = (status() == "firing" || status() == "all")
                        .then(|| platform_section(&page.platform));
                    let nothing = page.platform.alerts.is_empty()
                        && page.platform.prometheus != "down"
                        && page.platform.grafana != "down";
                    view! {
                        {platform}
                        <p class="vc-muted" id="alerts-empty" hidden=!nothing>
                            {move_tr!("alerts-empty")}
                        </p>
                    }
                        .into_any()
                }
                Some(Ok(page)) => {
                    let platform = (status() == "firing" || status() == "all")
                        .then(|| platform_section(&page.platform));
                    view! {
                        {platform}
                        <div class="vc-card">
                            <table class="vc-table" id="alerts-table">
                                <thead>
                                    <tr>
                                        <th>{move_tr!("alerts-col-severity")}</th>
                                        <th>{move_tr!("alerts-col-rule")}</th>
                                        <th>{move_tr!("alerts-col-device")}</th>
                                        <th>{move_tr!("alerts-col-value")}</th>
                                        <th>{move_tr!("alerts-col-fired")}</th>
                                        <th>{move_tr!("alerts-col-status")}</th>
                                        <th></th>
                                    </tr>
                                </thead>
                                <tbody>
                                    {page
                                        .alerts
                                        .into_iter()
                                        .map(|a| {
                                            let id = a.id;
                                            let firing = a.status == "firing";
                                            let trailing = match a.status.as_str() {
                                                "resolved" => shown(&a.resolved_at),
                                                "acknowledged" => {
                                                    format!("{} {}", shown(&a.acknowledged_at), a.acknowledged_by)
                                                }
                                                _ => String::new(),
                                            };
                                            view! {
                                                <tr data-id=id data-status=a.status.clone()>
                                                    <td>
                                                        <span
                                                            class="vc-badge"
                                                            class:vc-badge--danger=a.severity == "critical"
                                                            class:vc-badge--warning=a.severity == "warning"
                                                        >
                                                            {severity_label(&a.severity)}
                                                        </span>
                                                    </td>
                                                    <td>{a.rule.clone()}</td>
                                                    <td>
                                                        <A href=format!("/automation/devices/{}", a.device_id)>{a.device.clone()}</A>
                                                    </td>
                                                    <td>{a.value.clone()}</td>
                                                    <td>{shown(&a.fired_at)}</td>
                                                    <td>
                                                        {status_label(&a.status)} <span class="vc-muted">{format!(" {trailing}")}</span>
                                                    </td>
                                                    <td>
                                                        {firing
                                                            .then(|| {
                                                                view! {
                                                                    <button
                                                                        type="button"
                                                                        class="vc-btn vc-btn--ghost vc-btn--sm alert-ack"
                                                                        disabled=move || !ready.get() || ack.pending().get()
                                                                        on:click=move |_| {
                                                                            ack.dispatch(id);
                                                                        }
                                                                    >
                                                                        {move_tr!("alerts-ack")}
                                                                    </button>
                                                                }
                                                            })}
                                                    </td>
                                                </tr>
                                            }
                                        })
                                        .collect_view()}
                                </tbody>
                            </table>
                        </div>
                    }
                        .into_any()
                }
            }}
        </Transition>
    }
}
