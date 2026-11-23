// SPDX-License-Identifier: AGPL-3.0-or-later
//! Administration of the Prometheus (`/admin/prometheus-rules`) and Grafana (`/admin/grafana-rules`) alert rules (R7 lot 2,
//! ADR 0052). Each modification rewrites the provisioning file and prompts the service to re-read it; a button redoes
//! both on demand and says what happened.

use dominet_proto::{
    DeleteOutcome, GrafanaRuleDto, GrafanaRuleInput, PrometheusRuleDto, PrometheusRuleInput,
    RuleSyncDto, SaveOutcome,
};
use leptos::prelude::*;
use leptos_fluent::move_tr;

use crate::crud::{CrudResult, CrudTable, FieldKind, FieldSpec, Labels, Row, Values};
use crate::guard::RequireAdmin;

// ----------------------------------------------------------------------------------------------------------- server

#[cfg(feature = "ssr")]
fn sync_dto(s: &dominet_server::rules::SyncStatus) -> RuleSyncDto {
    use dominet_server::rules::SyncStatus;
    let (state, groups, detail) = match s {
        SyncStatus::Disabled => ("disabled", 0, String::new()),
        SyncStatus::Done { groups, reloaded } => (
            match reloaded {
                Some(true) => "ok",
                Some(false) => "reload_failed",
                None => "no_url",
            },
            u32::try_from(*groups).unwrap_or(u32::MAX),
            String::new(),
        ),
        SyncStatus::Failed(why) => ("failed", 0, why.clone()),
    };
    RuleSyncDto {
        state: state.into(),
        groups,
        detail,
    }
}

#[cfg(feature = "ssr")]
fn prom_dto(r: &dominet_domain::observability::PrometheusRule) -> PrometheusRuleDto {
    let d = &r.draft;
    PrometheusRuleDto {
        id: r.id.0,
        name: d.name.clone(),
        group: d.group.clone(),
        severity: d.severity.as_str().into(),
        expr: d.expr.clone(),
        for_duration: d.for_duration.clone(),
        summary: d.summary.clone(),
        description: d.description.clone(),
        eval_interval: d.eval_interval.clone(),
        active: d.active,
    }
}

#[cfg(feature = "ssr")]
fn graf_dto(r: &dominet_domain::observability::GrafanaRule) -> GrafanaRuleDto {
    let d = &r.draft;
    GrafanaRuleDto {
        id: r.id.0,
        uid: d.uid.clone(),
        title: d.title.clone(),
        folder: d.folder.clone(),
        group: d.group.clone(),
        group_interval: d.group_interval.clone(),
        datasource_uid: d.datasource_uid.clone(),
        expr: d.expr.clone(),
        query_window: d.query_window,
        for_duration: d.for_duration.clone(),
        severity: d.severity.as_str().into(),
        summary: d.summary.clone(),
        description: d.description.clone(),
        no_data_state: d.no_data_state.clone(),
        exec_err_state: d.exec_err_state.clone(),
        active: d.active,
    }
}

/// Prometheus rules, by group then name (administrators).
///
/// # Errors
/// Refused (403) for a non-administrator; internal error if the database fails.
#[server(name = ListPrometheusRules, prefix = "/sf", endpoint = "prometheus-rules/list")]
pub async fn list_prometheus_rules() -> Result<Vec<PrometheusRuleDto>, ServerFnError> {
    use dominet_domain::repo::PrometheusRuleRepo;

    use crate::server_fns::{internal, require_admin};

    let state = expect_context::<dominet_server::AppState>();
    require_admin(&state).await?;
    let rules = state
        .store
        .prometheus_rules()
        .list()
        .await
        .map_err(|e| internal("Prometheus rules", &e))?;
    Ok(rules.iter().map(prom_dto).collect())
}

/// Creates (`id` absent) or replaces (`id` present) a Prometheus rule, then rewrites `rules.yml` and reloads Prometheus.
///
/// # Errors
/// Refused (403) for a non-administrator; internal error if the database fails.
#[server(name = SavePrometheusRule, prefix = "/sf", endpoint = "prometheus-rules/save", input = server_fn::codec::Json)]
pub async fn save_prometheus_rule(
    input: PrometheusRuleInput,
) -> Result<SaveOutcome<PrometheusRuleDto>, ServerFnError> {
    use dominet_domain::Problem;
    use dominet_domain::alert::Severity;
    use dominet_domain::observability::{PrometheusRuleDraft, PrometheusRuleId};

    use crate::server_fns::{field_problems, internal, require_admin};

    let state = expect_context::<dominet_server::AppState>();
    let admin = require_admin(&state).await?;
    let mut problems = vec![];
    let severity = Severity::parse(&input.severity).unwrap_or_else(|| {
        problems.push(("severity", Problem::Invalid));
        Severity::Warning
    });
    let draft = PrometheusRuleDraft {
        name: input.name.trim().to_string(),
        group: input.group.trim().to_string(),
        severity,
        expr: input.expr.trim().to_string(),
        for_duration: input.for_duration.trim().to_string(),
        summary: input.summary.trim().to_string(),
        description: input.description.trim().to_string(),
        eval_interval: input.eval_interval.trim().to_string(),
        active: input.active,
    };
    problems.extend(draft.problems());
    if !problems.is_empty() {
        return Ok(SaveOutcome::Invalid(field_problems(problems)));
    }
    let saved = dominet_server::config_service::save_prometheus_rule(
        &state,
        input.id.map(PrometheusRuleId),
        &draft,
    )
    .await
    .map_err(|e| internal("saving a Prometheus rule", &e))?;
    Ok(match saved {
        SaveOutcome::Saved(rule) => {
            tracing::info!(
                admin_id = admin.id.0,
                rule_id = rule.id.0,
                created = input.id.is_none(),
                "Prometheus rule saved"
            );
            SaveOutcome::Saved(prom_dto(&rule))
        }
        SaveOutcome::Invalid(p) => SaveOutcome::Invalid(p),
        SaveOutcome::NotFound => SaveOutcome::NotFound,
        SaveOutcome::Blocked(b) => SaveOutcome::Blocked(b),
    })
}

/// Deletes a Prometheus rule, then rewrites `rules.yml` and reloads Prometheus.
///
/// # Errors
/// Refused (403) for a non-administrator; internal error if the database fails.
#[server(name = DeletePrometheusRule, prefix = "/sf", endpoint = "prometheus-rules/delete")]
pub async fn delete_prometheus_rule(id: i64) -> Result<DeleteOutcome, ServerFnError> {
    use dominet_domain::observability::PrometheusRuleId;

    use crate::server_fns::{internal, require_admin};

    let state = expect_context::<dominet_server::AppState>();
    let admin = require_admin(&state).await?;
    let out = dominet_server::config_service::delete_prometheus_rule(&state, PrometheusRuleId(id))
        .await
        .map_err(|e| internal("deleting a Prometheus rule", &e))?;
    if out == DeleteOutcome::Deleted {
        tracing::info!(
            admin_id = admin.id.0,
            rule_id = id,
            "Prometheus rule deleted"
        );
    }
    Ok(out)
}

/// Grafana rules, by folder, group then title (administrators).
///
/// # Errors
/// Refused (403) for a non-administrator; internal error if the database fails.
#[server(name = ListGrafanaRules, prefix = "/sf", endpoint = "grafana-rules/list")]
pub async fn list_grafana_rules() -> Result<Vec<GrafanaRuleDto>, ServerFnError> {
    use dominet_domain::repo::GrafanaRuleRepo;

    use crate::server_fns::{internal, require_admin};

    let state = expect_context::<dominet_server::AppState>();
    require_admin(&state).await?;
    let rules = state
        .store
        .grafana_rules()
        .list()
        .await
        .map_err(|e| internal("Grafana rules", &e))?;
    Ok(rules.iter().map(graf_dto).collect())
}

/// Creates (`id` absent) or replaces (`id` present) a Grafana rule, then rewrites `rules.yaml` and reloads Grafana.
///
/// # Errors
/// Refused (403) for a non-administrator; internal error if the database fails.
#[server(name = SaveGrafanaRule, prefix = "/sf", endpoint = "grafana-rules/save", input = server_fn::codec::Json)]
pub async fn save_grafana_rule(
    input: GrafanaRuleInput,
) -> Result<SaveOutcome<GrafanaRuleDto>, ServerFnError> {
    use dominet_domain::Problem;
    use dominet_domain::alert::Severity;
    use dominet_domain::observability::{GrafanaRuleDraft, GrafanaRuleId};

    use crate::server_fns::{field_problems, internal, require_admin};

    let state = expect_context::<dominet_server::AppState>();
    let admin = require_admin(&state).await?;
    let mut problems = vec![];
    let severity = Severity::parse(&input.severity).unwrap_or_else(|| {
        problems.push(("severity", Problem::Invalid));
        Severity::Warning
    });
    let draft = GrafanaRuleDraft {
        uid: input.uid.trim().to_string(),
        title: input.title.trim().to_string(),
        folder: input.folder.trim().to_string(),
        group: input.group.trim().to_string(),
        group_interval: input.group_interval.trim().to_string(),
        datasource_uid: input.datasource_uid.trim().to_string(),
        expr: input.expr.trim().to_string(),
        query_window: input.query_window,
        for_duration: input.for_duration.trim().to_string(),
        severity,
        summary: input.summary.trim().to_string(),
        description: input.description.trim().to_string(),
        no_data_state: input.no_data_state.trim().to_string(),
        exec_err_state: input.exec_err_state.trim().to_string(),
        active: input.active,
    };
    problems.extend(draft.problems());
    if !problems.is_empty() {
        return Ok(SaveOutcome::Invalid(field_problems(problems)));
    }
    let saved = dominet_server::config_service::save_grafana_rule(
        &state,
        input.id.map(GrafanaRuleId),
        &draft,
    )
    .await
    .map_err(|e| internal("saving a Grafana rule", &e))?;
    Ok(match saved {
        SaveOutcome::Saved(rule) => {
            tracing::info!(
                admin_id = admin.id.0,
                rule_id = rule.id.0,
                created = input.id.is_none(),
                "Grafana rule saved"
            );
            SaveOutcome::Saved(graf_dto(&rule))
        }
        SaveOutcome::Invalid(p) => SaveOutcome::Invalid(p),
        SaveOutcome::NotFound => SaveOutcome::NotFound,
        SaveOutcome::Blocked(b) => SaveOutcome::Blocked(b),
    })
}

/// Deletes a Grafana rule, then rewrites `rules.yaml` and reloads Grafana.
///
/// # Errors
/// Refused (403) for a non-administrator; internal error if the database fails.
#[server(name = DeleteGrafanaRule, prefix = "/sf", endpoint = "grafana-rules/delete")]
pub async fn delete_grafana_rule(id: i64) -> Result<DeleteOutcome, ServerFnError> {
    use dominet_domain::observability::GrafanaRuleId;

    use crate::server_fns::{internal, require_admin};

    let state = expect_context::<dominet_server::AppState>();
    let admin = require_admin(&state).await?;
    let out = dominet_server::config_service::delete_grafana_rule(&state, GrafanaRuleId(id))
        .await
        .map_err(|e| internal("deleting a Grafana rule", &e))?;
    if out == DeleteOutcome::Deleted {
        tracing::info!(admin_id = admin.id.0, rule_id = id, "Grafana rule deleted");
    }
    Ok(out)
}

/// Rewrites the rules file of `service` (`prometheus` or `grafana`) and reloads the service; says what happened.
///
/// # Errors
/// Refused (403) for a non-administrator; internal error for an unknown service.
#[server(name = SyncRules, prefix = "/sf", endpoint = "rules/sync")]
pub async fn sync_rules(service: String) -> Result<RuleSyncDto, ServerFnError> {
    use crate::server_fns::require_admin;

    let state = expect_context::<dominet_server::AppState>();
    require_admin(&state).await?;
    let status = match service.as_str() {
        "prometheus" => state.rules.sync_prometheus().await,
        "grafana" => state.rules.sync_grafana().await,
        _ => return Err(ServerFnError::new("unknown service")),
    };
    Ok(sync_dto(&status))
}

// ----------------------------------------------------------------------------------------------------------- interface

const SEVERITIES: &[(&str, &str)] = &[
    ("info", tk!("alerts-severity-info")),
    ("warning", tk!("alerts-severity-warning")),
    ("critical", tk!("alerts-severity-critical")),
];
const NO_DATA: &[(&str, &str)] = &[
    ("NoData", tk!("rules-state-nodata")),
    ("Alerting", tk!("rules-state-alerting")),
    ("OK", tk!("rules-state-ok")),
];
const EXEC_ERR: &[(&str, &str)] = &[
    ("Error", tk!("rules-state-error")),
    ("Alerting", tk!("rules-state-alerting")),
    ("OK", tk!("rules-state-ok")),
];

fn prom_row(r: &PrometheusRuleDto) -> Row {
    Row {
        id: r.id,
        values: Values::from([
            ("name", r.name.clone()),
            ("group", r.group.clone()),
            ("severity", r.severity.clone()),
            ("expr", r.expr.clone()),
            ("for_duration", r.for_duration.clone()),
            ("summary", r.summary.clone()),
            ("description", r.description.clone()),
            ("eval_interval", r.eval_interval.clone()),
            ("active", r.active.to_string()),
        ]),
    }
}

fn prom_fields() -> Vec<FieldSpec> {
    use FieldKind as K;
    vec![
        FieldSpec::new("name", tk!("rules-name"), K::Text)
            .required()
            .max(128)
            .column(),
        FieldSpec::new("group", tk!("rules-group"), K::Text)
            .required()
            .max(64)
            .default_value("dominet_custom")
            .column(),
        FieldSpec::new("severity", tk!("rules-severity"), K::Select(SEVERITIES))
            .default_value("warning")
            .required()
            .column(),
        FieldSpec::new("expr", tk!("rules-promql"), K::TextArea)
            .required()
            .max(4000)
            .hint(tk!("rules-promql-hint")),
        FieldSpec::new("for_duration", tk!("rules-for"), K::Text)
            .default_value("5m")
            .max(16)
            .hint(tk!("rules-duration-hint"))
            .column(),
        FieldSpec::new("summary", tk!("rules-summary"), K::Text)
            .required()
            .max(256),
        FieldSpec::new("description", tk!("rules-description"), K::TextArea)
            .max(2000)
            .hint(tk!("rules-description-hint")),
        FieldSpec::new("eval_interval", tk!("rules-eval-interval"), K::Text)
            .max(16)
            .hint(tk!("rules-eval-interval-hint")),
        FieldSpec::new("active", tk!("rules-active"), K::Checkbox)
            .default_value("true")
            .column(),
    ]
}

fn graf_row(r: &GrafanaRuleDto) -> Row {
    Row {
        id: r.id,
        values: Values::from([
            ("uid", r.uid.clone()),
            ("title", r.title.clone()),
            ("folder", r.folder.clone()),
            ("group", r.group.clone()),
            ("group_interval", r.group_interval.clone()),
            ("datasource_uid", r.datasource_uid.clone()),
            ("expr", r.expr.clone()),
            ("query_window", r.query_window.to_string()),
            ("for_duration", r.for_duration.clone()),
            ("severity", r.severity.clone()),
            ("summary", r.summary.clone()),
            ("description", r.description.clone()),
            ("no_data_state", r.no_data_state.clone()),
            ("exec_err_state", r.exec_err_state.clone()),
            ("active", r.active.to_string()),
        ]),
    }
}

fn graf_fields() -> Vec<FieldSpec> {
    use FieldKind as K;
    vec![
        FieldSpec::new("title", tk!("rules-title-field"), K::Text)
            .required()
            .max(128)
            .column(),
        FieldSpec::new("uid", tk!("rules-uid"), K::Text)
            .max(64)
            .hint(tk!("rules-uid-hint"))
            .column(),
        FieldSpec::new("folder", tk!("rules-folder"), K::Text)
            .required()
            .max(64)
            .default_value("DomiNet Alerts"),
        FieldSpec::new("group", tk!("rules-group"), K::Text)
            .required()
            .max(64)
            .default_value("DomiNet"),
        FieldSpec::new("group_interval", tk!("rules-group-interval"), K::Text)
            .default_value("1m")
            .max(16)
            .hint(tk!("rules-duration-hint")),
        FieldSpec::new("datasource_uid", tk!("rules-datasource"), K::Text)
            .required()
            .max(64)
            .default_value("prometheus")
            .hint(tk!("rules-datasource-hint"))
            .column(),
        FieldSpec::new("expr", tk!("rules-query"), K::TextArea)
            .required()
            .max(4000)
            .hint(tk!("rules-query-hint")),
        FieldSpec::new(
            "query_window",
            tk!("rules-query-window"),
            K::Number {
                min: 1,
                max: 86_400,
            },
        )
        .default_value("300")
        .hint(tk!("rules-query-window-hint")),
        FieldSpec::new("for_duration", tk!("rules-for"), K::Text)
            .default_value("5m")
            .max(16)
            .hint(tk!("rules-duration-hint")),
        FieldSpec::new("severity", tk!("rules-severity"), K::Select(SEVERITIES))
            .default_value("warning")
            .required()
            .column(),
        FieldSpec::new("summary", tk!("rules-summary"), K::Text)
            .required()
            .max(256),
        FieldSpec::new("description", tk!("rules-description"), K::TextArea).max(2000),
        FieldSpec::new("no_data_state", tk!("rules-no-data"), K::Select(NO_DATA))
            .default_value("NoData"),
        FieldSpec::new(
            "exec_err_state",
            tk!("rules-exec-error"),
            K::Select(EXEC_ERR),
        )
        .default_value("Error"),
        FieldSpec::new("active", tk!("rules-active"), K::Checkbox)
            .default_value("true")
            .column(),
    ]
}

/// "Write the files and reload" button and result, under the table.
#[component]
fn SyncCard(service: &'static str) -> impl IntoView {
    let result = RwSignal::new(None::<Result<RuleSyncDto, ()>>);
    let run = Action::new(move |(): &()| async move {
        result.set(Some(sync_rules(service.to_string()).await.map_err(|_| ())));
    });
    let message = move || {
        result.get().map(|r| match r {
            Err(()) => (false, move_tr!("rules-sync-error").get()),
            Ok(dto) => {
                let ok = matches!(dto.state.as_str(), "ok" | "no_url" | "disabled");
                let text = match dto.state.as_str() {
                    "ok" => move_tr!("rules-sync-ok").get(),
                    "no_url" => move_tr!("rules-sync-no-url").get(),
                    "reload_failed" => move_tr!("rules-sync-reload-failed").get(),
                    "disabled" => move_tr!("rules-sync-disabled").get(),
                    _ => move_tr!("rules-sync-failed").get(),
                };
                (ok, text)
            }
        })
    };
    view! {
        <section class="vc-card" id=format!("rules-sync-{service}")>
            <p class="vc-muted">{move_tr!("rules-sync-hint")}</p>
            <p class="vc-btn-row">
                <button type="button" class="vc-btn vc-btn--sm" id=format!("rules-sync-{service}-run") on:click=move |_| { run.dispatch(()); }>
                    {move_tr!("rules-sync-run")}
                </button>
            </p>
            {move || message().map(|(ok, text)| view! {
                <div class=if ok { "vc-alert vc-alert--success" } else { "vc-alert vc-alert--error" } role="status" id=format!("rules-sync-{service}-result")>{text}</div>
            })}
        </section>
    }
}

#[component]
pub fn PrometheusRulesPage() -> impl IntoView {
    view! {
        <RequireAdmin>
            <main class="vc-container">
                <PrometheusRulesAdmin/>
            </main>
        </RequireAdmin>
    }
}

#[component]
fn PrometheusRulesAdmin() -> impl IntoView {
    let version = RwSignal::new(0u32);
    let list = Resource::new(
        move || version.get(),
        |_| async { list_prometheus_rules().await.map_err(|_| ()) },
    );
    let rows = Signal::derive(move || match list.get() {
        Some(Ok(items)) => items.iter().map(prom_row).collect(),
        _ => vec![],
    });
    let save = Action::new(|(id, v): &(Option<i64>, Values)| {
        let text = |k: &str| v.get(k).cloned().unwrap_or_default();
        let input = PrometheusRuleInput {
            id: *id,
            name: text("name"),
            group: text("group"),
            severity: text("severity"),
            expr: text("expr"),
            for_duration: text("for_duration"),
            summary: text("summary"),
            description: text("description"),
            eval_interval: text("eval_interval"),
            active: v.get("active").is_some_and(|a| a == "true"),
        };
        async move {
            save_prometheus_rule(input)
                .await
                .map_or(CrudResult::Failed, CrudResult::from)
        }
    });
    let remove = Action::new(|id: &i64| {
        let id = *id;
        async move {
            delete_prometheus_rule(id)
                .await
                .map_or(CrudResult::Failed, CrudResult::from)
        }
    });
    view! {
        <Transition fallback=|| ()>
            {move || {
                matches!(list.get(), Some(Err(()))).then(|| {
                    view! { <div class="vc-alert vc-alert--error" role="alert" id="crud-load-error">{move_tr!("crud-load-error")}</div> }
                })
            }}
            <CrudTable
                labels=Labels {
                    title: tk!("prom-rules-title"),
                    add: tk!("prom-rules-add"),
                    edit: tk!("prom-rules-edit"),
                    empty: tk!("prom-rules-empty"),
                }
                fields=prom_fields()
                rows
                save
                remove
                on_change=Callback::new(move |()| version.update(|v| *v += 1))
            />
        </Transition>
        <SyncCard service="prometheus"/>
    }
}

#[component]
pub fn GrafanaRulesPage() -> impl IntoView {
    view! {
        <RequireAdmin>
            <main class="vc-container">
                <GrafanaRulesAdmin/>
            </main>
        </RequireAdmin>
    }
}

#[component]
fn GrafanaRulesAdmin() -> impl IntoView {
    let version = RwSignal::new(0u32);
    let list = Resource::new(
        move || version.get(),
        |_| async { list_grafana_rules().await.map_err(|_| ()) },
    );
    let rows = Signal::derive(move || match list.get() {
        Some(Ok(items)) => items.iter().map(graf_row).collect(),
        _ => vec![],
    });
    let save = Action::new(|(id, v): &(Option<i64>, Values)| {
        let text = |k: &str| v.get(k).cloned().unwrap_or_default();
        let input = GrafanaRuleInput {
            id: *id,
            uid: text("uid"),
            title: text("title"),
            folder: text("folder"),
            group: text("group"),
            group_interval: text("group_interval"),
            datasource_uid: text("datasource_uid"),
            expr: text("expr"),
            query_window: text("query_window").trim().parse().unwrap_or(0),
            for_duration: text("for_duration"),
            severity: text("severity"),
            summary: text("summary"),
            description: text("description"),
            no_data_state: text("no_data_state"),
            exec_err_state: text("exec_err_state"),
            active: v.get("active").is_some_and(|a| a == "true"),
        };
        async move {
            save_grafana_rule(input)
                .await
                .map_or(CrudResult::Failed, CrudResult::from)
        }
    });
    let remove = Action::new(|id: &i64| {
        let id = *id;
        async move {
            delete_grafana_rule(id)
                .await
                .map_or(CrudResult::Failed, CrudResult::from)
        }
    });
    view! {
        <Transition fallback=|| ()>
            {move || {
                matches!(list.get(), Some(Err(()))).then(|| {
                    view! { <div class="vc-alert vc-alert--error" role="alert" id="crud-load-error">{move_tr!("crud-load-error")}</div> }
                })
            }}
            <CrudTable
                labels=Labels {
                    title: tk!("graf-rules-title"),
                    add: tk!("graf-rules-add"),
                    edit: tk!("graf-rules-edit"),
                    empty: tk!("graf-rules-empty"),
                }
                fields=graf_fields()
                rows
                save
                remove
                on_change=Callback::new(move |()| version.update(|v| *v += 1))
            />
        </Transition>
        <SyncCard service="grafana"/>
    }
}
