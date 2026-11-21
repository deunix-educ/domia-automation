// SPDX-License-Identifier: AGPL-3.0-or-later
//! Administration des automations : `/admin/automations` (R4 lot 3, ADR 0032). Un declencheur, une condition facultative, une
//! action ; le formulaire n'affiche que les champs du type choisi. Le journal d'execution recent est sous le tableau.
//!
//! Les declencheurs `mqtt_message` et `device_state` sont executes par le moteur des ce lot ; `schedule` et `solar` le seront
//! avec le planificateur (lot 4), les scripts avec le lot 5.

use dominet_proto::{
    AutomationDto, AutomationFields, AutomationInput, AutomationRunDto, DeleteOutcome, SaveOutcome,
};
use leptos::prelude::*;
use leptos_fluent::{I18n, move_tr, tr};

use crate::crud::{CrudResult, CrudTable, FieldKind, FieldSpec, Labels, Row, Values};
use crate::guard::RequireAdmin;

// ------------------------------------------------------------------ conversions (serveur)

#[cfg(feature = "ssr")]
fn fields_of(a: &dominet_domain::automation::Automation) -> AutomationFields {
    use dominet_domain::automation::{Action, Trigger, value_to_text};

    let d = &a.draft;
    let mut f = AutomationFields {
        name: d.name.clone(),
        enabled: d.enabled,
        description: d.description.clone(),
        trigger_type: d.trigger.kind().to_string(),
        action_type: d.action.kind().to_string(),
        script: d.script.clone(),
        ..AutomationFields::default()
    };
    match &d.trigger {
        Trigger::MqttMessage {
            topic,
            payload_filter,
        } => {
            f.topic.clone_from(topic);
            if !payload_filter.is_empty() {
                f.payload_filter = serde_json::Value::Object(payload_filter.clone()).to_string();
            }
        }
        Trigger::Schedule { cron } => f.cron.clone_from(cron),
        Trigger::Solar {
            event,
            latitude,
            longitude,
        } => {
            f.event = event.as_str().to_string();
            f.latitude = latitude.to_string();
            f.longitude = longitude.to_string();
        }
        Trigger::DeviceState {
            friendly_name,
            key,
            value,
        } => {
            f.friendly_name.clone_from(friendly_name);
            f.key.clone_from(key);
            f.value = value_to_text(value);
        }
    }
    if let Some(c) = &d.condition {
        f.condition_key.clone_from(&c.key);
        f.condition_value = value_to_text(&c.value);
    }
    match &d.action {
        Action::MqttPublish { topic, payload } => {
            f.action_topic.clone_from(topic);
            f.action_payload = payload.to_string();
        }
        Action::Notification { message, sms } => {
            f.message.clone_from(message);
            f.sms = *sms;
        }
        Action::Scene { scene } => f.scene_id = scene.0.to_string(),
        Action::Script => {}
    }
    f
}

#[cfg(feature = "ssr")]
fn summaries(a: &dominet_domain::automation::Automation) -> (String, String) {
    use dominet_domain::automation::{Action, Trigger, value_to_text};

    let trigger = match &a.draft.trigger {
        Trigger::MqttMessage { topic, .. } => format!("mqtt_message {topic}"),
        Trigger::Schedule { cron } => format!("schedule {cron}"),
        Trigger::Solar { event, .. } => format!("solar {}", event.as_str()),
        Trigger::DeviceState {
            friendly_name,
            key,
            value,
        } => format!(
            "device_state {friendly_name} : {key} = {}",
            value_to_text(value)
        ),
    };
    let action = match &a.draft.action {
        Action::MqttPublish { topic, .. } => format!("mqtt_publish {topic}"),
        Action::Script => "script".to_string(),
        Action::Notification { .. } => "notification".to_string(),
        Action::Scene { scene } => format!("scene #{}", scene.0),
    };
    (trigger, action)
}

#[cfg(feature = "ssr")]
fn dto_of(a: &dominet_domain::automation::Automation, next_run: String) -> AutomationDto {
    let (trigger_summary, action_summary) = summaries(a);
    AutomationDto {
        id: a.id.0,
        fields: fields_of(a),
        trigger_summary,
        action_summary,
        trigger_count: a.trigger_count,
        last_triggered: a
            .last_triggered
            .map(dominet_server::timefmt::text)
            .unwrap_or_default(),
        next_run,
    }
}

/// Prochain passage de chaque automation planifiee, en heure locale du serveur.
#[cfg(feature = "ssr")]
async fn next_runs(
    state: &dominet_server::AppState,
) -> Result<std::collections::HashMap<i64, String>, ServerFnError> {
    use dominet_domain::repo::ScheduleRepo;

    Ok(state
        .store
        .schedule()
        .automation_next_runs()
        .await
        .map_err(|e| crate::server_fns::internal("next runs", &e))?
        .into_iter()
        .filter_map(|(id, at)| at.map(|t| (id.0, dominet_server::timefmt::text(t))))
        .collect())
}

/// Lit un nombre saisi avec point ou virgule decimale.
#[cfg(feature = "ssr")]
fn decimal(text: &str) -> Option<f64> {
    text.trim()
        .replace(',', ".")
        .parse::<f64>()
        .ok()
        .filter(|v| v.is_finite())
}

/// Brouillon depuis le formulaire, ou les problemes champ par champ.
#[cfg(feature = "ssr")]
fn draft_of(
    f: &AutomationFields,
) -> Result<dominet_domain::automation::AutomationDraft, dominet_domain::automation::Problems> {
    use dominet_domain::Problem;
    use dominet_domain::automation::{AutomationDraft, value_from_text};
    use serde_json::{Value, json};

    let mut problems: dominet_domain::automation::Problems = vec![];
    let json_object = |field: &'static str,
                       text: &str,
                       empty: Value,
                       out: &mut dominet_domain::automation::Problems| {
        if text.trim().is_empty() {
            return empty;
        }
        if let Ok(v @ Value::Object(_)) = serde_json::from_str::<Value>(text) {
            v
        } else {
            out.push((field, Problem::Invalid));
            empty
        }
    };
    let trigger_config = match f.trigger_type.as_str() {
        "mqtt_message" => {
            let mut cfg = json!({"topic": f.topic.trim()});
            let filter = json_object(
                "payload_filter",
                &f.payload_filter,
                json!({}),
                &mut problems,
            );
            if filter.as_object().is_some_and(|o| !o.is_empty()) {
                cfg["payload_filter"] = filter;
            }
            cfg
        }
        "schedule" => json!({"cron": f.cron.trim()}),
        "solar" => {
            let number = |field: &'static str,
                          text: &str,
                          out: &mut dominet_domain::automation::Problems| {
                decimal(text).unwrap_or_else(|| {
                    out.push((
                        field,
                        if text.trim().is_empty() {
                            Problem::Required
                        } else {
                            Problem::Invalid
                        },
                    ));
                    0.0
                })
            };
            let latitude = number("latitude", &f.latitude, &mut problems);
            let longitude = number("longitude", &f.longitude, &mut problems);
            json!({"event": f.event.trim(), "latitude": latitude, "longitude": longitude})
        }
        "device_state" => json!({
            "friendly_name": f.friendly_name.trim(),
            "key": f.key.trim(),
            "value": value_from_text(&f.value),
        }),
        _ => Value::Null,
    };
    let condition = if f.condition_key.trim().is_empty() {
        json!({})
    } else {
        json!({"key": f.condition_key.trim(), "value": value_from_text(&f.condition_value)})
    };
    let action_config = match f.action_type.as_str() {
        "mqtt_publish" => json!({
            "topic": f.action_topic.trim(),
            "payload": json_object("action_payload", &f.action_payload, json!({}), &mut problems),
        }),
        "notification" => json!({"message": f.message.trim(), "sms": f.sms}),
        "scene" => json!({"scene_id": f.scene_id.trim().parse::<i64>().unwrap_or(0)}),
        _ => json!({}),
    };
    match AutomationDraft::from_parts(
        f.name.trim(),
        f.enabled,
        f.description.trim(),
        &f.trigger_type,
        &trigger_config,
        &condition,
        &f.action_type,
        &action_config,
        &f.script,
    ) {
        Ok(draft) if problems.is_empty() => Ok(draft),
        Ok(_) => Err(problems),
        Err(mut more) => {
            problems.append(&mut more);
            // Le champ du formulaire pour la condition s'appelle `condition_key`.
            for p in &mut problems {
                if p.0 == "condition" {
                    p.0 = "condition_key";
                }
            }
            // Un champ illisible en JSON est signale une fois, pas aussi par la validation de sa valeur de repli.
            let mut seen = std::collections::HashSet::new();
            problems.retain(|(field, _)| seen.insert(*field));
            Err(problems)
        }
    }
}

// ------------------------------------------------------------------ fonctions serveur

/// Automations, par nom (administrateurs).
///
/// # Errors
/// Refus (403) pour un non-administrateur ; erreur interne si la base echoue.
#[server(name = ListAutomations, prefix = "/sf", endpoint = "automations/list")]
pub async fn list_automations() -> Result<Vec<AutomationDto>, ServerFnError> {
    use dominet_domain::repo::AutomationRepo;

    use crate::server_fns::{internal, require_admin};

    let state = expect_context::<dominet_server::AppState>();
    require_admin(&state).await?;
    let items = state
        .store
        .automations()
        .list()
        .await
        .map_err(|e| internal("automation list", &e))?;
    let next = next_runs(&state).await?;
    Ok(items
        .iter()
        .map(|a| dto_of(a, next.get(&a.id.0).cloned().unwrap_or_default()))
        .collect())
}

/// Cree (`id` absent) ou remplace (`id` present) une automation. Les champs invalides sont renvoyes un par un ; le moteur
/// recharge ses automations et ses abonnements tout de suite.
///
/// # Errors
/// Refus (403) pour un non-administrateur ; erreur interne si la base echoue.
#[server(name = SaveAutomation, prefix = "/sf", endpoint = "automations/save", input = server_fn::codec::Json)]
pub async fn save_automation(
    input: AutomationInput,
) -> Result<SaveOutcome<AutomationDto>, ServerFnError> {
    use dominet_domain::automation::AutomationId;

    use crate::server_fns::{field_problems, internal, require_admin};

    let state = expect_context::<dominet_server::AppState>();
    let admin = require_admin(&state).await?;
    let draft = match draft_of(&input.fields) {
        Ok(d) => d,
        Err(problems) => return Ok(SaveOutcome::Invalid(field_problems(problems))),
    };
    // Le script doit se compiler et la scene visee exister : ces controles, comme le rechargement du moteur, sont dans le service
    // partage avec le serveur MCP.
    match dominet_server::config_service::save_automation(
        &state,
        input.id.map(AutomationId),
        &draft,
    )
    .await
    .map_err(|e| internal("saving an automation", &e))?
    {
        SaveOutcome::Saved(a) => {
            tracing::info!(
                admin_id = admin.id.0,
                automation_id = a.id.0,
                created = input.id.is_none(),
                trigger = a.draft.trigger.kind(),
                action = a.draft.action.kind(),
                "automation saved"
            );
            Ok(SaveOutcome::Saved(dto_of(&a, String::new())))
        }
        SaveOutcome::Invalid(p) => Ok(SaveOutcome::Invalid(p)),
        SaveOutcome::NotFound => Ok(SaveOutcome::NotFound),
        SaveOutcome::Blocked(b) => Ok(SaveOutcome::Blocked(b)),
    }
}

/// Supprime une automation (son journal avec elle).
///
/// # Errors
/// Refus (403) pour un non-administrateur ; erreur interne si la base echoue.
#[server(name = DeleteAutomation, prefix = "/sf", endpoint = "automations/delete")]
pub async fn delete_automation(id: i64) -> Result<DeleteOutcome, ServerFnError> {
    use dominet_domain::automation::AutomationId;

    use crate::server_fns::{internal, require_admin};

    let state = expect_context::<dominet_server::AppState>();
    let admin = require_admin(&state).await?;
    let outcome = dominet_server::config_service::delete_automation(&state, AutomationId(id))
        .await
        .map_err(|e| internal("suppression d'une automation", &e))?;
    if outcome == DeleteOutcome::Deleted {
        tracing::info!(
            admin_id = admin.id.0,
            automation_id = id,
            "automation deleted"
        );
    }
    Ok(outcome)
}

/// Les 50 dernieres executions, toutes automations confondues, les plus recentes d'abord (administrateurs).
///
/// # Errors
/// Refus (403) pour un non-administrateur ; erreur interne si la base echoue.
#[server(name = AutomationRuns, prefix = "/sf", endpoint = "automations/runs")]
pub async fn automation_runs() -> Result<Vec<AutomationRunDto>, ServerFnError> {
    use std::collections::HashMap;

    use dominet_domain::repo::{AutomationRepo, AutomationRunRepo};

    use crate::server_fns::{internal, require_admin};

    let state = expect_context::<dominet_server::AppState>();
    require_admin(&state).await?;
    let names: HashMap<i64, String> = state
        .store
        .automations()
        .list()
        .await
        .map_err(|e| internal("automation list", &e))?
        .into_iter()
        .map(|a| (a.id.0, a.draft.name))
        .collect();
    let runs = state
        .store
        .automation_runs()
        .recent_all(50)
        .await
        .map_err(|e| internal("journal des automations", &e))?;
    Ok(runs
        .into_iter()
        .map(|r| AutomationRunDto {
            automation_id: r.automation.0,
            automation: names.get(&r.automation.0).cloned().unwrap_or_default(),
            at: dominet_server::timefmt::text(r.at),
            trigger: r.trigger,
            status: r.status.as_str().to_string(),
            duration_ms: r.duration_ms,
            message: r.message,
        })
        .collect())
}

// ------------------------------------------------------------------ page

const TRIGGERS: &[(&str, &str)] = &[
    ("device_state", tk!("autos-trigger-device-state")),
    ("mqtt_message", tk!("autos-trigger-mqtt-message")),
    ("schedule", tk!("autos-trigger-schedule")),
    ("solar", tk!("autos-trigger-solar")),
];

const ACTIONS: &[(&str, &str)] = &[
    ("mqtt_publish", tk!("autos-action-mqtt-publish")),
    ("scene", tk!("autos-action-scene")),
    ("notification", tk!("autos-action-notification")),
    ("script", tk!("autos-action-script")),
];

const SOLAR_EVENTS: &[(&str, &str)] = &[
    ("sunrise", tk!("autos-solar-sunrise")),
    ("sunset", tk!("autos-solar-sunset")),
    ("solar_noon", tk!("autos-solar-noon")),
    ("dawn_civil", tk!("autos-solar-dawn-civil")),
    ("dusk_civil", tk!("autos-solar-dusk-civil")),
    ("dawn_nautical", tk!("autos-solar-dawn-nautical")),
    ("dusk_nautical", tk!("autos-solar-dusk-nautical")),
    ("dawn_astronomical", tk!("autos-solar-dawn-astronomical")),
    ("dusk_astronomical", tk!("autos-solar-dusk-astronomical")),
];

fn row(a: &AutomationDto) -> Row {
    let f = &a.fields;
    Row {
        id: a.id,
        values: Values::from([
            ("name", f.name.clone()),
            ("enabled", f.enabled.to_string()),
            ("description", f.description.clone()),
            ("trigger_type", f.trigger_type.clone()),
            ("topic", f.topic.clone()),
            ("payload_filter", f.payload_filter.clone()),
            ("friendly_name", f.friendly_name.clone()),
            ("key", f.key.clone()),
            ("value", f.value.clone()),
            ("cron", f.cron.clone()),
            ("event", f.event.clone()),
            ("latitude", f.latitude.clone()),
            ("longitude", f.longitude.clone()),
            ("condition_key", f.condition_key.clone()),
            ("condition_value", f.condition_value.clone()),
            ("action_type", f.action_type.clone()),
            ("action_topic", f.action_topic.clone()),
            ("action_payload", f.action_payload.clone()),
            ("message", f.message.clone()),
            ("sms", f.sms.to_string()),
            ("scene_id", f.scene_id.clone()),
            ("script", f.script.clone()),
            ("trigger_summary", a.trigger_summary.clone()),
            ("action_summary", a.action_summary.clone()),
            ("trigger_count", a.trigger_count.to_string()),
            ("last_triggered", a.last_triggered.clone()),
            ("next_run", a.next_run.clone()),
        ]),
    }
}

fn fields() -> Vec<FieldSpec> {
    let text = |name, label| FieldSpec::new(name, label, FieldKind::Text);
    vec![
        text("name", tk!("autos-name")).required().max(128).column(),
        FieldSpec::new("trigger_summary", tk!("autos-trigger"), FieldKind::Display).column(),
        FieldSpec::new("action_summary", tk!("autos-action"), FieldKind::Display).column(),
        FieldSpec::new("enabled", tk!("autos-enabled"), FieldKind::Checkbox)
            .default_value("true")
            .column(),
        FieldSpec::new("trigger_count", tk!("autos-count"), FieldKind::Display).column(),
        FieldSpec::new("last_triggered", tk!("autos-last"), FieldKind::Display).column(),
        FieldSpec::new("next_run", tk!("autos-next"), FieldKind::Display).column(),
        text("description", tk!("autos-description")).max(256),
        FieldSpec::new(
            "trigger_type",
            tk!("autos-trigger"),
            FieldKind::Select(TRIGGERS),
        )
        .default_value("device_state")
        .required(),
        FieldSpec::new("topic", tk!("autos-topic"), FieldKind::Text)
            .max(256)
            .hint(tk!("autos-topic-hint")),
        FieldSpec::new(
            "payload_filter",
            tk!("autos-payload-filter"),
            FieldKind::TextArea,
        )
        .hint(tk!("autos-payload-filter-hint")),
        FieldSpec::new("friendly_name", tk!("autos-device"), FieldKind::Choice),
        text("key", tk!("autos-key")).max(64),
        text("value", tk!("autos-value")).hint(tk!("autos-value-hint")),
        text("cron", tk!("autos-cron")).hint(tk!("autos-cron-hint")),
        FieldSpec::new("event", tk!("autos-event"), FieldKind::Select(SOLAR_EVENTS))
            .default_value("sunset"),
        text("latitude", tk!("autos-latitude")),
        text("longitude", tk!("autos-longitude")),
        text("condition_key", tk!("autos-condition-key"))
            .max(64)
            .hint(tk!("autos-condition-hint")),
        text("condition_value", tk!("autos-condition-value")),
        FieldSpec::new(
            "action_type",
            tk!("autos-action"),
            FieldKind::Select(ACTIONS),
        )
        .default_value("scene")
        .required(),
        text("action_topic", tk!("autos-action-topic")).max(256),
        FieldSpec::new(
            "action_payload",
            tk!("autos-action-payload"),
            FieldKind::TextArea,
        )
        .default_value("{}")
        .hint(tk!("autos-action-payload-hint")),
        text("message", tk!("autos-message")).max(500),
        FieldSpec::new("sms", tk!("autos-sms"), FieldKind::Checkbox),
        FieldSpec::new("scene_id", tk!("autos-scene"), FieldKind::Choice),
        FieldSpec::new("script", tk!("autos-script"), FieldKind::Code)
            .max(20_000)
            .hint(tk!("autos-script-hint")),
    ]
}

/// Champs visibles selon le type de declencheur et d'action choisis.
fn visible(field: &'static str, v: &Values) -> bool {
    let trigger = v.get("trigger_type").map_or("", String::as_str);
    let action = v.get("action_type").map_or("", String::as_str);
    match field {
        "topic" | "payload_filter" => trigger == "mqtt_message",
        "friendly_name" | "key" | "value" => trigger == "device_state",
        "cron" => trigger == "schedule",
        "event" | "latitude" | "longitude" => trigger == "solar",
        "action_topic" | "action_payload" => action == "mqtt_publish",
        "message" | "sms" => action == "notification",
        "scene_id" => action == "scene",
        "script" => action == "script",
        _ => true,
    }
}

#[component]
pub fn AutomationsAdminPage() -> impl IntoView {
    view! {
        <RequireAdmin>
            <main class="vc-container">
                <AutomationsAdmin/>
            </main>
        </RequireAdmin>
    }
}

fn fields_from(v: &Values) -> AutomationFields {
    let text = |k: &str| v.get(k).cloned().unwrap_or_default();
    let flag = |k: &str| v.get(k).is_some_and(|a| a == "true");
    AutomationFields {
        name: text("name"),
        enabled: flag("enabled"),
        description: text("description"),
        trigger_type: text("trigger_type"),
        topic: text("topic"),
        payload_filter: text("payload_filter"),
        friendly_name: text("friendly_name"),
        key: text("key"),
        value: text("value"),
        cron: text("cron"),
        event: text("event"),
        latitude: text("latitude"),
        longitude: text("longitude"),
        condition_key: text("condition_key"),
        condition_value: text("condition_value"),
        action_type: text("action_type"),
        action_topic: text("action_topic"),
        action_payload: text("action_payload"),
        message: text("message"),
        sms: flag("sms"),
        scene_id: text("scene_id"),
        script: text("script"),
    }
}

#[component]
fn AutomationsAdmin() -> impl IntoView {
    crate::devices_admin::provide_choices(&["device_name", "scene_id"]);
    let version = RwSignal::new(0u32);
    let list = Resource::new(
        move || version.get(),
        |_| async { list_automations().await.map_err(|_| ()) },
    );
    let rows = Signal::derive(move || match list.get() {
        Some(Ok(items)) => items.iter().map(row).collect(),
        _ => vec![],
    });
    let save = Action::new(|(id, v): &(Option<i64>, Values)| {
        let input = AutomationInput {
            id: *id,
            fields: fields_from(v),
        };
        async move {
            save_automation(input)
                .await
                .map_or(CrudResult::Failed, CrudResult::from)
        }
    });
    let remove = Action::new(|id: &i64| {
        let id = *id;
        async move {
            delete_automation(id)
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
                    title: tk!("autos-title"),
                    add: tk!("autos-add"),
                    edit: tk!("autos-edit"),
                    empty: tk!("autos-empty"),
                }
                fields=fields()
                rows
                save
                remove
                on_change=Callback::new(move |()| version.update(|v| *v += 1))
                field_visible=Callback::new(|(name, v): (&'static str, Values)| visible(name, &v))
            />
        </Transition>
        <RunsJournal version/>
    }
}

/// Journal d'execution recent ; il se recharge avec le bouton, ou quand la liste change.
#[component]
fn RunsJournal(version: RwSignal<u32>) -> impl IntoView {
    let i18n = expect_context::<I18n>();
    let refresh = RwSignal::new(0u32);
    let runs = Resource::new(
        move || (version.get(), refresh.get()),
        |_| async { automation_runs().await.map_err(|_| ()) },
    );
    view! {
        <section class="vc-card" id="runs-journal">
            <div class="vc-page-header">
                <h3 class="vc-card-title">{move_tr!("autos-journal")}</h3>
                <button type="button" id="runs-refresh" class="vc-btn vc-btn--ghost vc-btn--sm" on:click=move |_| refresh.update(|v| *v += 1)>
                    {move_tr!("autos-journal-refresh")}
                </button>
            </div>
            <Transition fallback=|| ()>
                {move || match runs.get() {
                    None => ().into_any(),
                    Some(Err(())) => view! { <p class="vc-field-error">{move_tr!("crud-load-error")}</p> }.into_any(),
                    Some(Ok(items)) if items.is_empty() => {
                        view! { <p class="vc-muted" id="runs-empty">{move_tr!("autos-journal-empty")}</p> }.into_any()
                    }
                    Some(Ok(items)) => view! {
                        <table class="vc-table" id="runs-table">
                            <thead>
                                <tr>
                                    <th>{move || i18n.tr(tk!("autos-run-at"))}</th>
                                    <th>{move || i18n.tr(tk!("autos-name"))}</th>
                                    <th>{move || i18n.tr(tk!("autos-run-trigger"))}</th>
                                    <th>{move || i18n.tr(tk!("autos-run-status"))}</th>
                                    <th>{move || i18n.tr(tk!("autos-run-message"))}</th>
                                </tr>
                            </thead>
                            <tbody>
                                {items
                                    .into_iter()
                                    .map(|r| {
                                        let ok = r.status == "ok";
                                        let bad = r.status == "error";
                                        view! {
                                            <tr data-status=r.status.clone()>
                                                <td>{r.at.clone()}</td>
                                                <td>{r.automation.clone()}</td>
                                                <td>{r.trigger.clone()}</td>
                                                <td>
                                                    <span class="vc-badge" class:vc-badge--success=ok class:vc-badge--danger=bad>
                                                        {status_text(&r.status)}
                                                    </span>
                                                </td>
                                                <td>{r.message.clone()}</td>
                                            </tr>
                                        }
                                    })
                                    .collect_view()}
                            </tbody>
                        </table>
                    }
                    .into_any(),
                }}
            </Transition>
        </section>
    }
}

fn status_text(status: &str) -> String {
    match status {
        "ok" => tr!("autos-status-ok"),
        "error" => tr!("autos-status-error"),
        "skipped" => tr!("autos-status-skipped"),
        _ => tr!("autos-status-limited"),
    }
}
