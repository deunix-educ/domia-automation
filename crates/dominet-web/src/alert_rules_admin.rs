// SPDX-License-Identifier: AGPL-3.0-or-later
//! Administration des regles d'alerte : `/admin/alert-rules` (R4 lot 6, ADR 0035). Une regle surveille une cle d'etat d'un appareil
//! et declenche une alerte quand la condition est vraie (cooldown, trois canaux d'envoi dont seul le bandeau est servi avant R5).

use dominet_proto::{AlertRuleDto, AlertRuleInput, DeleteOutcome, SaveOutcome};
use leptos::prelude::*;
use leptos_fluent::move_tr;

use crate::crud::{CrudResult, CrudTable, FieldKind, FieldSpec, Labels, Row, Values};
use crate::guard::RequireAdmin;

/// Lit un nombre saisi avec point ou virgule decimale ; vide = absent.
#[cfg(feature = "ssr")]
fn decimal(text: &str) -> Result<Option<f64>, ()> {
    let t = text.trim();
    if t.is_empty() {
        return Ok(None);
    }
    t.replace(',', ".")
        .parse::<f64>()
        .ok()
        .filter(|v| v.is_finite())
        .map(Some)
        .ok_or(())
}

/// Regles d'alerte avec le nombre d'alertes en cours de chacune (administrateurs).
///
/// # Errors
/// Refus (403) pour un non-administrateur ; erreur interne si la base echoue.
#[server(name = ListAlertRules, prefix = "/sf", endpoint = "alert-rules/list")]
pub async fn list_alert_rules() -> Result<Vec<AlertRuleDto>, ServerFnError> {
    use std::collections::HashMap;

    use dominet_domain::alert::AlertStatus;
    use dominet_domain::repo::{AlertRepo, AlertRuleRepo, DeviceFilter, DeviceRepo};

    use crate::server_fns::{internal, require_admin};

    let state = expect_context::<dominet_server::AppState>();
    require_admin(&state).await?;
    let rules = state
        .store
        .alert_rules()
        .list()
        .await
        .map_err(|e| internal("alert rules", &e))?;
    let devices: HashMap<i64, String> = state
        .store
        .devices()
        .list(&DeviceFilter::default())
        .await
        .map_err(|e| internal("device list", &e))?
        .into_iter()
        .map(|d| (d.id.0, d.friendly_name))
        .collect();
    let mut firing: HashMap<i64, u32> = HashMap::new();
    for a in state
        .store
        .alerts()
        .list(Some(AlertStatus::Firing), 5000)
        .await
        .map_err(|e| internal("current alerts", &e))?
    {
        if let Some(r) = a.rule {
            *firing.entry(r.0).or_default() += 1;
        }
    }
    Ok(rules
        .into_iter()
        .map(|r| {
            let d = r.rule;
            AlertRuleDto {
                id: r.id.0,
                name: d.name,
                device_id: d.device.0.to_string(),
                device: devices.get(&d.device.0).cloned().unwrap_or_default(),
                key: d.key,
                operator: d.operator.as_str().to_string(),
                threshold_num: d.threshold_num.map(|t| t.to_string()).unwrap_or_default(),
                threshold_str: d.threshold_str,
                severity: d.severity.as_str().to_string(),
                cooldown_minutes: d.cooldown_minutes.to_string(),
                notify_email: d.notify_email,
                notify_sms: d.notify_sms,
                notify_android: d.notify_android,
                enabled: d.enabled,
                description: d.description,
                firing: firing.get(&r.id.0).copied().unwrap_or(0),
            }
        })
        .collect())
}

/// Cree (`id` absent) ou remplace (`id` present) une regle. Les champs invalides sont renvoyes un par un ; l'evaluation relit
/// les regles tout de suite.
///
/// # Errors
/// Refus (403) pour un non-administrateur ; erreur interne si la base echoue.
#[server(name = SaveAlertRule, prefix = "/sf", endpoint = "alert-rules/save", input = server_fn::codec::Json)]
pub async fn save_alert_rule(
    input: AlertRuleInput,
) -> Result<SaveOutcome<AlertRuleDto>, ServerFnError> {
    use dominet_domain::Problem;
    use dominet_domain::alert::{AlertRuleDraft, AlertRuleId, Operator, Severity};
    use dominet_domain::device::DeviceId;

    use crate::server_fns::{field_problems, internal, require_admin};

    let state = expect_context::<dominet_server::AppState>();
    let admin = require_admin(&state).await?;
    let mut problems = vec![];
    let device = match input.device_id.trim().parse::<i64>() {
        Ok(id) if id > 0 => id,
        _ => {
            problems.push(("device_id", Problem::Required));
            0
        }
    };
    let mut draft = AlertRuleDraft::new(input.name.trim(), DeviceId(device), input.key.trim());
    draft.operator = Operator::parse(&input.operator).unwrap_or_else(|| {
        problems.push(("operator", Problem::Invalid));
        Operator::Gt
    });
    draft.severity = Severity::parse(&input.severity).unwrap_or_else(|| {
        problems.push(("severity", Problem::Invalid));
        Severity::Warning
    });
    match decimal(&input.threshold_num) {
        Ok(v) => draft.threshold_num = v,
        Err(()) => problems.push(("threshold_num", Problem::Invalid)),
    }
    draft.threshold_str = input.threshold_str.trim().to_string();
    match input.cooldown_minutes.trim().parse::<i16>() {
        Ok(v) => draft.cooldown_minutes = v,
        Err(_) => problems.push((
            "cooldown_minutes",
            Problem::OutOfRange { min: 0, max: 1440 },
        )),
    }
    draft.notify_email = input.notify_email;
    draft.notify_sms = input.notify_sms;
    draft.notify_android = input.notify_android;
    draft.enabled = input.enabled;
    draft.description = input.description.trim().to_string();
    for p in draft.problems() {
        if !problems.iter().any(|(f, _)| *f == p.0) {
            problems.push(p);
        }
    }
    if !problems.is_empty() {
        return Ok(SaveOutcome::Invalid(field_problems(problems)));
    }
    match dominet_server::config_service::save_alert_rule(&state, input.id.map(AlertRuleId), &draft)
        .await
        .map_err(|e| internal("saving an alert rule", &e))?
    {
        SaveOutcome::Saved(rule) => {
            tracing::info!(
                admin_id = admin.id.0,
                rule_id = rule.id.0,
                created = input.id.is_none(),
                "alert rule saved"
            );
            let dto = list_alert_rules_inner(&state)
                .await?
                .into_iter()
                .find(|r| r.id == rule.id.0);
            Ok(dto.map_or(SaveOutcome::NotFound, SaveOutcome::Saved))
        }
        SaveOutcome::Invalid(p) => Ok(SaveOutcome::Invalid(p)),
        SaveOutcome::NotFound => Ok(SaveOutcome::NotFound),
        SaveOutcome::Blocked(b) => Ok(SaveOutcome::Blocked(b)),
    }
}

#[cfg(feature = "ssr")]
async fn list_alert_rules_inner(
    _state: &dominet_server::AppState,
) -> Result<Vec<AlertRuleDto>, ServerFnError> {
    list_alert_rules().await
}

/// Supprime une regle ; les alertes deja creees restent (sans regle).
///
/// # Errors
/// Refus (403) pour un non-administrateur ; erreur interne si la base echoue.
#[server(name = DeleteAlertRule, prefix = "/sf", endpoint = "alert-rules/delete")]
pub async fn delete_alert_rule(id: i64) -> Result<DeleteOutcome, ServerFnError> {
    use dominet_domain::alert::AlertRuleId;

    use crate::server_fns::{internal, require_admin};

    let state = expect_context::<dominet_server::AppState>();
    let admin = require_admin(&state).await?;
    let outcome = dominet_server::config_service::delete_alert_rule(&state, AlertRuleId(id))
        .await
        .map_err(|e| internal("deleting an alert rule", &e))?;
    if outcome == DeleteOutcome::Deleted {
        tracing::info!(admin_id = admin.id.0, rule_id = id, "alert rule deleted");
    }
    Ok(outcome)
}

const OPERATORS: &[(&str, &str)] = &[
    ("gt", tk!("alert-rules-op-gt")),
    ("gte", tk!("alert-rules-op-gte")),
    ("lt", tk!("alert-rules-op-lt")),
    ("lte", tk!("alert-rules-op-lte")),
    ("eq", tk!("alert-rules-op-eq")),
    ("neq", tk!("alert-rules-op-neq")),
    ("contains", tk!("alert-rules-op-contains")),
];

const SEVERITIES: &[(&str, &str)] = &[
    ("info", tk!("alerts-severity-info")),
    ("warning", tk!("alerts-severity-warning")),
    ("critical", tk!("alerts-severity-critical")),
];

fn row(r: &AlertRuleDto) -> Row {
    Row {
        id: r.id,
        values: Values::from([
            ("name", r.name.clone()),
            ("device_id", r.device_id.clone()),
            ("key", r.key.clone()),
            ("operator", r.operator.clone()),
            ("threshold_num", r.threshold_num.clone()),
            ("threshold_str", r.threshold_str.clone()),
            ("severity", r.severity.clone()),
            ("cooldown_minutes", r.cooldown_minutes.clone()),
            ("notify_email", r.notify_email.to_string()),
            ("notify_sms", r.notify_sms.to_string()),
            ("notify_android", r.notify_android.to_string()),
            ("enabled", r.enabled.to_string()),
            ("description", r.description.clone()),
            ("firing", r.firing.to_string()),
        ]),
    }
}

fn fields() -> Vec<FieldSpec> {
    vec![
        FieldSpec::new("name", tk!("alert-rules-name"), FieldKind::Text)
            .required()
            .max(128)
            .column(),
        FieldSpec::new("device_id", tk!("alert-rules-device"), FieldKind::Choice).column(),
        FieldSpec::new("key", tk!("alert-rules-key"), FieldKind::Text)
            .required()
            .max(64)
            .hint(tk!("alert-rules-key-hint"))
            .column(),
        FieldSpec::new(
            "operator",
            tk!("alert-rules-operator"),
            FieldKind::Select(OPERATORS),
        )
        .default_value("gt")
        .required()
        .column(),
        FieldSpec::new(
            "threshold_num",
            tk!("alert-rules-threshold-num"),
            FieldKind::Text,
        )
        .hint(tk!("alert-rules-threshold-hint")),
        FieldSpec::new(
            "threshold_str",
            tk!("alert-rules-threshold-str"),
            FieldKind::Text,
        )
        .max(128),
        FieldSpec::new(
            "severity",
            tk!("alert-rules-severity"),
            FieldKind::Select(SEVERITIES),
        )
        .default_value("warning")
        .required()
        .column(),
        FieldSpec::new(
            "cooldown_minutes",
            tk!("alert-rules-cooldown"),
            FieldKind::Number { min: 0, max: 1440 },
        )
        .default_value("10")
        .hint(tk!("alert-rules-cooldown-hint")),
        FieldSpec::new(
            "notify_email",
            tk!("alert-rules-email"),
            FieldKind::Checkbox,
        )
        .default_value("true"),
        FieldSpec::new("notify_sms", tk!("alert-rules-sms"), FieldKind::Checkbox),
        FieldSpec::new(
            "notify_android",
            tk!("alert-rules-android"),
            FieldKind::Checkbox,
        )
        .default_value("true")
        .hint(tk!("alert-rules-android-hint")),
        FieldSpec::new("enabled", tk!("alert-rules-enabled"), FieldKind::Checkbox)
            .default_value("true")
            .column(),
        FieldSpec::new(
            "description",
            tk!("alert-rules-description"),
            FieldKind::Text,
        )
        .max(256),
        FieldSpec::new("firing", tk!("alert-rules-firing"), FieldKind::Display).column(),
    ]
}

#[component]
pub fn AlertRulesAdminPage() -> impl IntoView {
    view! {
        <RequireAdmin>
            <main class="vc-container">
                <AlertRulesAdmin/>
            </main>
        </RequireAdmin>
    }
}

#[component]
fn AlertRulesAdmin() -> impl IntoView {
    crate::devices_admin::provide_choices(&["device_id"]);
    let version = RwSignal::new(0u32);
    let list = Resource::new(
        move || version.get(),
        |_| async { list_alert_rules().await.map_err(|_| ()) },
    );
    let rows = Signal::derive(move || match list.get() {
        Some(Ok(items)) => items.iter().map(row).collect(),
        _ => vec![],
    });
    let save = Action::new(|(id, v): &(Option<i64>, Values)| {
        let text = |k: &str| v.get(k).cloned().unwrap_or_default();
        let flag = |k: &str| v.get(k).is_some_and(|a| a == "true");
        let input = AlertRuleInput {
            id: *id,
            name: text("name"),
            device_id: text("device_id"),
            key: text("key"),
            operator: text("operator"),
            threshold_num: text("threshold_num"),
            threshold_str: text("threshold_str"),
            severity: text("severity"),
            cooldown_minutes: text("cooldown_minutes"),
            notify_email: flag("notify_email"),
            notify_sms: flag("notify_sms"),
            notify_android: flag("notify_android"),
            enabled: flag("enabled"),
            description: text("description"),
        };
        async move {
            save_alert_rule(input)
                .await
                .map_or(CrudResult::Failed, CrudResult::from)
        }
    });
    let remove = Action::new(|id: &i64| {
        let id = *id;
        async move {
            delete_alert_rule(id)
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
                    title: tk!("alert-rules-title"),
                    add: tk!("alert-rules-add"),
                    edit: tk!("alert-rules-edit"),
                    empty: tk!("alert-rules-empty"),
                }
                fields=fields()
                rows
                save
                remove
                on_change=Callback::new(move |()| version.update(|v| *v += 1))
            />
        </Transition>
    }
}
