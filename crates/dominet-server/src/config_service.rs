// SPDX-License-Identifier: AGPL-3.0-or-later
//! Saving and deleting configuration, **one code path for the interface and the MCP server** (R11 M4, ADR 0058).
//!
//! What goes beyond the domain draft lives here, once: a Rhai script must compile, the scene an automation targets must exist,
//! name conflicts become field problems, and the running parts (automation engine, scheduler, alert evaluation) are told to
//! reload. The callers keep what is theirs: the interface turns form strings into a draft, the MCP tools turn JSON into one.
//!
//! The functions return the same [`SaveOutcome`] / [`DeleteOutcome`] the interface already speaks: expected refusals are values,
//! only a storage failure is an error.

use dominet_domain::RepoError;
use dominet_domain::alert::{AlertRule, AlertRuleDraft, AlertRuleId};
use dominet_domain::automation::{Action, Automation, AutomationDraft, AutomationId};
use dominet_domain::dashboard::{
    Dashboard, DashboardDraft, DashboardId, Widget, WidgetDraft, WidgetId,
};
use dominet_domain::device::{
    Device, DeviceDraft, DeviceGroup, DeviceGroupDraft, DeviceGroupId, DeviceId, DeviceLabel,
    DeviceLabelDraft, DeviceLabelId, MqttSource, MqttSourceId, WeatherStation, WeatherStationId,
};
use dominet_domain::location::{Location, LocationDraft, LocationId};
use dominet_domain::navbar::{NavbarDraft, NavbarItem, NavbarItemId};
use dominet_domain::observability::{
    GrafanaRule, GrafanaRuleDraft, GrafanaRuleId, PrometheusRule, PrometheusRuleDraft,
    PrometheusRuleId,
};
use dominet_domain::repo::{
    AlertRuleRepo, AutomationRepo, DashboardRepo, DetectorConfigRepo, DeviceGroupRepo,
    DeviceLabelRepo, DeviceRepo, GrafanaRuleRepo, LocationRepo, MqttSourceRepo, NavbarRepo,
    PrometheusRuleRepo, SceneRepo, WeatherRepo, WidgetRepo,
};
use dominet_domain::scene::{Scene, SceneDraft, SceneId};
use dominet_domain::user::UserId;
use dominet_proto::{DeleteOutcome, FieldProblem, ProblemKind, SaveOutcome};

use crate::AppState;

/// Problems of the domain, field by field, as the interface shows them.
#[must_use]
pub fn field_problems(problems: Vec<(&'static str, dominet_domain::Problem)>) -> Vec<FieldProblem> {
    use dominet_domain::Problem;
    problems
        .into_iter()
        .map(|(field, p)| FieldProblem {
            field: field.to_string(),
            kind: match p {
                Problem::Required => ProblemKind::Required,
                Problem::TooLong { max } => ProblemKind::TooLong {
                    max: u32::try_from(max).unwrap_or(u32::MAX),
                },
                Problem::Invalid => ProblemKind::Invalid,
                Problem::OutOfRange { min, max } => ProblemKind::OutOfRange { min, max },
            },
        })
        .collect()
}

fn one(field: &str, kind: ProblemKind) -> Vec<FieldProblem> {
    vec![FieldProblem {
        field: field.to_string(),
        kind,
    }]
}

// ------------------------------------------------------------------ locations

/// Creates (`id` absent) or replaces a location.
///
/// # Errors
/// Storage failure.
pub async fn save_location(
    state: &AppState,
    id: Option<LocationId>,
    draft: &LocationDraft,
) -> Result<SaveOutcome<Location>, RepoError> {
    let problems = draft.problems();
    if !problems.is_empty() {
        return Ok(SaveOutcome::Invalid(field_problems(problems)));
    }
    let repo = state.store.locations();
    let saved = match id {
        None => repo.create(draft).await,
        Some(id) => repo.update(id, draft).await,
    };
    match saved {
        Ok(l) => Ok(SaveOutcome::Saved(l)),
        Err(RepoError::NotFound) => Ok(SaveOutcome::NotFound),
        Err(e) => Err(e),
    }
}

/// Deletes a location; `InUse` while something still points to it.
///
/// # Errors
/// Storage failure.
pub async fn delete_location(state: &AppState, id: LocationId) -> Result<DeleteOutcome, RepoError> {
    match state.store.locations().delete(id).await {
        Ok(()) => Ok(DeleteOutcome::Deleted),
        Err(RepoError::NotFound) => Ok(DeleteOutcome::NotFound),
        Err(RepoError::Conflict { .. }) => Ok(DeleteOutcome::InUse),
        Err(e) => Err(e),
    }
}

// ------------------------------------------------------------------ scenes

/// Creates or replaces a scene (its actions with it, in one transaction). The caller has already read its input; the domain
/// problems of the draft are checked here.
///
/// # Errors
/// Storage failure.
pub async fn save_scene(
    state: &AppState,
    id: Option<SceneId>,
    draft: &SceneDraft,
) -> Result<SaveOutcome<Scene>, RepoError> {
    let problems = draft.problems();
    if !problems.is_empty() {
        return Ok(SaveOutcome::Invalid(field_problems(problems)));
    }
    let repo = state.store.scenes();
    let saved = match id {
        None => repo.create(draft).await,
        Some(id) => repo.update(id, draft).await,
    };
    match saved {
        Ok(scene) => Ok(SaveOutcome::Saved(scene)),
        Err(RepoError::NotFound) => Ok(SaveOutcome::NotFound),
        Err(RepoError::Conflict { constraint }) => {
            let (field, kind) = if constraint.contains("name") {
                ("name", ProblemKind::Taken)
            } else if constraint.contains("location") {
                ("location_id", ProblemKind::Invalid)
            } else {
                ("actions", ProblemKind::Invalid)
            };
            Ok(SaveOutcome::Invalid(one(field, kind)))
        }
        Err(e) => Err(e),
    }
}

/// Deletes a scene: the widgets that pointed to it lose it, the automations that run it fail (and say so in their journal).
///
/// # Errors
/// Storage failure.
pub async fn delete_scene(state: &AppState, id: SceneId) -> Result<DeleteOutcome, RepoError> {
    match state.store.scenes().delete(id).await {
        Ok(()) => Ok(DeleteOutcome::Deleted),
        Err(RepoError::NotFound) => Ok(DeleteOutcome::NotFound),
        Err(e) => Err(e),
    }
}

// ------------------------------------------------------------------ automations

/// What an automation draft needs beyond its own validation: a script must compile (the parser's message, with its line, comes
/// back on the `script` field) and the scene an action targets must exist (the key is only a number in the action's JSON: the
/// database cannot check it). Empty when all is well. A dry run asks this too, so it announces the refusals a real save would give.
///
/// # Errors
/// Storage failure.
pub async fn check_automation(
    state: &AppState,
    draft: &AutomationDraft,
) -> Result<Vec<FieldProblem>, RepoError> {
    if draft.action == Action::Script
        && let Err(detail) = dominet_automation::script::check(&draft.script)
    {
        return Ok(one("script", ProblemKind::Syntax(detail)));
    }
    if let Action::Scene { scene } = &draft.action
        && state.store.scenes().get(*scene).await?.is_none()
    {
        return Ok(one("scene_id", ProblemKind::Invalid));
    }
    Ok(vec![])
}

/// Creates or replaces an automation, then tells the engine and the scheduler to reload (after [`check_automation`]).
///
/// # Errors
/// Storage failure.
pub async fn save_automation(
    state: &AppState,
    id: Option<AutomationId>,
    draft: &AutomationDraft,
) -> Result<SaveOutcome<Automation>, RepoError> {
    let problems = check_automation(state, draft).await?;
    if !problems.is_empty() {
        return Ok(SaveOutcome::Invalid(problems));
    }
    let repo = state.store.automations();
    let saved = match id {
        None => repo.create(draft).await,
        Some(id) => repo.update(id, draft).await,
    };
    match saved {
        Ok(a) => {
            state.automations_updated();
            Ok(SaveOutcome::Saved(a))
        }
        Err(RepoError::NotFound) => Ok(SaveOutcome::NotFound),
        Err(e) => Err(e),
    }
}

/// Deletes an automation (its journal with it) and tells the engine and the scheduler.
///
/// # Errors
/// Storage failure.
pub async fn delete_automation(
    state: &AppState,
    id: AutomationId,
) -> Result<DeleteOutcome, RepoError> {
    match state.store.automations().delete(id).await {
        Ok(()) => {
            state.automations_updated();
            Ok(DeleteOutcome::Deleted)
        }
        Err(RepoError::NotFound) => Ok(DeleteOutcome::NotFound),
        Err(e) => Err(e),
    }
}

// ------------------------------------------------------------------ alert rules

/// Creates or replaces a device alert rule, then tells the alert evaluation to reload.
///
/// # Errors
/// Storage failure.
pub async fn save_alert_rule(
    state: &AppState,
    id: Option<AlertRuleId>,
    draft: &AlertRuleDraft,
) -> Result<SaveOutcome<AlertRule>, RepoError> {
    let problems = draft.problems();
    if !problems.is_empty() {
        return Ok(SaveOutcome::Invalid(field_problems(problems)));
    }
    let repo = state.store.alert_rules();
    let saved = match id {
        None => repo.create(draft).await,
        Some(id) => repo.update(id, draft).await,
    };
    match saved {
        Ok(rule) => {
            state.alert_rules_changed.notify_one();
            Ok(SaveOutcome::Saved(rule))
        }
        Err(RepoError::NotFound) => Ok(SaveOutcome::NotFound),
        // The device does not exist.
        Err(RepoError::Conflict { .. }) => {
            Ok(SaveOutcome::Invalid(one("device_id", ProblemKind::Invalid)))
        }
        Err(e) => Err(e),
    }
}

/// Deletes a rule; the alerts already raised stay (without a rule).
///
/// # Errors
/// Storage failure.
pub async fn delete_alert_rule(
    state: &AppState,
    id: AlertRuleId,
) -> Result<DeleteOutcome, RepoError> {
    match state.store.alert_rules().delete(id).await {
        Ok(()) => {
            state.alert_rules_changed.notify_one();
            Ok(DeleteOutcome::Deleted)
        }
        Err(RepoError::NotFound) => Ok(DeleteOutcome::NotFound),
        Err(e) => Err(e),
    }
}

// ------------------------------------------------------------------ device groups and labels

/// Creates or replaces a device group.
///
/// # Errors
/// Storage failure.
pub async fn save_device_group(
    state: &AppState,
    id: Option<DeviceGroupId>,
    draft: &DeviceGroupDraft,
) -> Result<SaveOutcome<DeviceGroup>, RepoError> {
    let problems = draft.problems();
    if !problems.is_empty() {
        return Ok(SaveOutcome::Invalid(field_problems(problems)));
    }
    let repo = state.store.device_groups();
    let saved = match id {
        None => repo.create(draft).await,
        Some(id) => repo.update(id, draft).await,
    };
    match saved {
        Ok(g) => Ok(SaveOutcome::Saved(g)),
        Err(RepoError::NotFound) => Ok(SaveOutcome::NotFound),
        Err(RepoError::Conflict { constraint }) => {
            let (field, kind) = if constraint.contains("location") {
                ("location_id", ProblemKind::Invalid)
            } else {
                ("name", ProblemKind::Taken)
            };
            Ok(SaveOutcome::Invalid(one(field, kind)))
        }
        Err(e) => Err(e),
    }
}

/// Deletes a group: its devices stay, without a group.
///
/// # Errors
/// Storage failure.
pub async fn delete_device_group(
    state: &AppState,
    id: DeviceGroupId,
) -> Result<DeleteOutcome, RepoError> {
    match state.store.device_groups().delete(id).await {
        Ok(()) => Ok(DeleteOutcome::Deleted),
        Err(RepoError::NotFound) => Ok(DeleteOutcome::NotFound),
        Err(e) => Err(e),
    }
}

/// Creates or replaces the label of a state key.
///
/// # Errors
/// Storage failure.
pub async fn save_device_label(
    state: &AppState,
    id: Option<DeviceLabelId>,
    draft: &DeviceLabelDraft,
) -> Result<SaveOutcome<DeviceLabel>, RepoError> {
    let problems = draft.problems();
    if !problems.is_empty() {
        return Ok(SaveOutcome::Invalid(field_problems(problems)));
    }
    let repo = state.store.device_labels();
    let saved = match id {
        None => repo.create(draft).await,
        Some(id) => repo.update(id, draft).await,
    };
    match saved {
        Ok(l) => Ok(SaveOutcome::Saved(l)),
        Err(RepoError::NotFound) => Ok(SaveOutcome::NotFound),
        Err(RepoError::Conflict { .. }) => Ok(SaveOutcome::Invalid(one("key", ProblemKind::Taken))),
        Err(e) => Err(e),
    }
}

/// Deletes a label.
///
/// # Errors
/// Storage failure.
pub async fn delete_device_label(
    state: &AppState,
    id: DeviceLabelId,
) -> Result<DeleteOutcome, RepoError> {
    match state.store.device_labels().delete(id).await {
        Ok(()) => Ok(DeleteOutcome::Deleted),
        Err(RepoError::NotFound) => Ok(DeleteOutcome::NotFound),
        Err(e) => Err(e),
    }
}

// ------------------------------------------------------------------ MQTT sources and weather stations

/// What a source draft needs beyond its own validation: its handler code, Rhai, must compile.
#[must_use]
pub fn check_mqtt_source(draft: &MqttSource) -> Vec<FieldProblem> {
    let mut problems = field_problems(draft.problems());
    if !draft.handler_code.trim().is_empty()
        && !problems.iter().any(|p| p.field == "handler_code")
        && let Err(detail) = dominet_automation::script::check(&draft.handler_code)
    {
        problems.push(FieldProblem {
            field: "handler_code".to_string(),
            kind: ProblemKind::Syntax(detail),
        });
    }
    problems
}

/// Creates or replaces an MQTT source, then tells the MQTT client to resubscribe.
///
/// # Errors
/// Storage failure.
pub async fn save_mqtt_source(
    state: &AppState,
    id: Option<MqttSourceId>,
    draft: &MqttSource,
) -> Result<SaveOutcome<MqttSource>, RepoError> {
    let problems = check_mqtt_source(draft);
    if !problems.is_empty() {
        return Ok(SaveOutcome::Invalid(problems));
    }
    let repo = state.store.mqtt_sources();
    let saved = match id {
        None => repo.create(draft).await,
        Some(id) => repo.update(id, draft).await,
    };
    match saved {
        Ok(s) => {
            state.sources_changed.notify_one();
            Ok(SaveOutcome::Saved(s))
        }
        Err(RepoError::NotFound) => Ok(SaveOutcome::NotFound),
        Err(e) => Err(e),
    }
}

/// Deletes a source (the devices it created stay) and tells the MQTT client.
///
/// # Errors
/// Storage failure.
pub async fn delete_mqtt_source(
    state: &AppState,
    id: MqttSourceId,
) -> Result<DeleteOutcome, RepoError> {
    match state.store.mqtt_sources().delete(id).await {
        Ok(()) => {
            state.sources_changed.notify_one();
            Ok(DeleteOutcome::Deleted)
        }
        Err(RepoError::NotFound) => Ok(DeleteOutcome::NotFound),
        Err(e) => Err(e),
    }
}

/// Creates or replaces a weather station, then tells the weather service.
///
/// # Errors
/// Storage failure.
pub async fn save_weather_station(
    state: &AppState,
    id: Option<WeatherStationId>,
    draft: &WeatherStation,
) -> Result<SaveOutcome<WeatherStation>, RepoError> {
    let problems = draft.problems();
    if !problems.is_empty() {
        return Ok(SaveOutcome::Invalid(field_problems(problems)));
    }
    let repo = state.store.weather();
    let saved = match id {
        None => repo.create(draft).await,
        Some(id) => repo.update(id, draft).await,
    };
    match saved {
        Ok(s) => {
            state.stations_changed.notify_one();
            Ok(SaveOutcome::Saved(s))
        }
        Err(RepoError::NotFound) => Ok(SaveOutcome::NotFound),
        Err(e) => Err(e),
    }
}

/// Deletes a station (the device it fed stays).
///
/// # Errors
/// Storage failure.
pub async fn delete_weather_station(
    state: &AppState,
    id: WeatherStationId,
) -> Result<DeleteOutcome, RepoError> {
    match state.store.weather().delete(id).await {
        Ok(()) => Ok(DeleteOutcome::Deleted),
        Err(RepoError::NotFound) => Ok(DeleteOutcome::NotFound),
        Err(e) => Err(e),
    }
}

// ------------------------------------------------------------------ Prometheus and Grafana rules

/// Creates or replaces a Prometheus rule, then rewrites `rules.yml` and reloads Prometheus (best effort: the status is on the
/// rules page, a failed reload does not undo the save).
///
/// # Errors
/// Storage failure.
pub async fn save_prometheus_rule(
    state: &AppState,
    id: Option<PrometheusRuleId>,
    draft: &PrometheusRuleDraft,
) -> Result<SaveOutcome<PrometheusRule>, RepoError> {
    let problems = draft.problems();
    if !problems.is_empty() {
        return Ok(SaveOutcome::Invalid(field_problems(problems)));
    }
    let repo = state.store.prometheus_rules();
    let saved = match id {
        None => repo.create(draft).await,
        Some(id) => repo.update(id, draft).await,
    };
    match saved {
        Ok(rule) => {
            let _ = state.rules.sync_prometheus().await;
            Ok(SaveOutcome::Saved(rule))
        }
        Err(RepoError::NotFound) => Ok(SaveOutcome::NotFound),
        // Only the name is unique.
        Err(RepoError::Conflict { .. }) => {
            Ok(SaveOutcome::Invalid(one("name", ProblemKind::Invalid)))
        }
        Err(e) => Err(e),
    }
}

/// Deletes a Prometheus rule, then rewrites `rules.yml` and reloads Prometheus.
///
/// # Errors
/// Storage failure.
pub async fn delete_prometheus_rule(
    state: &AppState,
    id: PrometheusRuleId,
) -> Result<DeleteOutcome, RepoError> {
    match state.store.prometheus_rules().delete(id).await {
        Ok(()) => {
            let _ = state.rules.sync_prometheus().await;
            Ok(DeleteOutcome::Deleted)
        }
        Err(RepoError::NotFound) => Ok(DeleteOutcome::NotFound),
        Err(e) => Err(e),
    }
}

/// Creates or replaces a Grafana rule, then rewrites `rules.yaml` and reloads Grafana (best effort, as for Prometheus).
///
/// # Errors
/// Storage failure.
pub async fn save_grafana_rule(
    state: &AppState,
    id: Option<GrafanaRuleId>,
    draft: &GrafanaRuleDraft,
) -> Result<SaveOutcome<GrafanaRule>, RepoError> {
    let problems = draft.problems();
    if !problems.is_empty() {
        return Ok(SaveOutcome::Invalid(field_problems(problems)));
    }
    let repo = state.store.grafana_rules();
    let saved = match id {
        None => repo.create(draft).await,
        Some(id) => repo.update(id, draft).await,
    };
    match saved {
        Ok(rule) => {
            let _ = state.rules.sync_grafana().await;
            Ok(SaveOutcome::Saved(rule))
        }
        Err(RepoError::NotFound) => Ok(SaveOutcome::NotFound),
        Err(RepoError::Conflict { .. }) => {
            Ok(SaveOutcome::Invalid(one("uid", ProblemKind::Invalid)))
        }
        Err(e) => Err(e),
    }
}

/// Deletes a Grafana rule, then rewrites `rules.yaml` and reloads Grafana.
///
/// # Errors
/// Storage failure.
pub async fn delete_grafana_rule(
    state: &AppState,
    id: GrafanaRuleId,
) -> Result<DeleteOutcome, RepoError> {
    match state.store.grafana_rules().delete(id).await {
        Ok(()) => {
            let _ = state.rules.sync_grafana().await;
            Ok(DeleteOutcome::Deleted)
        }
        Err(RepoError::NotFound) => Ok(DeleteOutcome::NotFound),
        Err(e) => Err(e),
    }
}

// ------------------------------------------------------------------ dashboards and widgets

/// Creates (with its `owner`) or renames a dashboard. The owner never changes afterwards; `is_default` can only be switched on.
///
/// # Errors
/// Storage failure.
pub async fn save_dashboard(
    state: &AppState,
    id: Option<DashboardId>,
    owner: Option<UserId>,
    draft: &DashboardDraft,
) -> Result<SaveOutcome<Dashboard>, RepoError> {
    let problems = draft.problems();
    if !problems.is_empty() {
        return Ok(SaveOutcome::Invalid(field_problems(problems)));
    }
    let repo = state.store.dashboards();
    let saved = match (id, owner) {
        (Some(id), _) => repo.update(id, draft).await,
        (None, Some(owner)) => repo.create(owner, draft).await,
        (None, None) => {
            return Ok(SaveOutcome::Invalid(one("owner", ProblemKind::Required)));
        }
    };
    match saved {
        Ok(d) => Ok(SaveOutcome::Saved(d)),
        Err(RepoError::NotFound) => Ok(SaveOutcome::NotFound),
        Err(e) => Err(e),
    }
}

/// Deletes a dashboard and its widgets.
///
/// # Errors
/// Storage failure.
pub async fn delete_dashboard(
    state: &AppState,
    id: DashboardId,
) -> Result<DeleteOutcome, RepoError> {
    match state.store.dashboards().delete(id).await {
        Ok(()) => Ok(DeleteOutcome::Deleted),
        Err(RepoError::NotFound) => Ok(DeleteOutcome::NotFound),
        Err(e) => Err(e),
    }
}

/// Creates a widget in `dashboard` (`id` absent) or replaces one. An overlap is reported on the `col` field, an unknown device or
/// scene on its own field.
///
/// # Errors
/// Storage failure.
pub async fn save_widget(
    state: &AppState,
    id: Option<WidgetId>,
    dashboard: DashboardId,
    draft: &WidgetDraft,
) -> Result<SaveOutcome<Widget>, RepoError> {
    let problems = draft.problems();
    if !problems.is_empty() {
        return Ok(SaveOutcome::Invalid(field_problems(problems)));
    }
    let repo = state.store.widgets();
    let saved = match id {
        None => repo.create(dashboard, draft).await,
        Some(id) => repo.update(id, draft).await,
    };
    match saved {
        Ok(w) => Ok(SaveOutcome::Saved(w)),
        Err(RepoError::NotFound) => Ok(SaveOutcome::NotFound),
        // Three constraints can refuse: an unknown device, an unknown scene, or the overlap.
        Err(RepoError::Conflict { constraint }) => {
            let (field, kind) = if constraint == "widget_device_fk" {
                ("device_id", ProblemKind::Invalid)
            } else if constraint == "widget_scene_fk" {
                ("scene_id", ProblemKind::Invalid)
            } else {
                ("col", ProblemKind::Overlap)
            };
            Ok(SaveOutcome::Invalid(one(field, kind)))
        }
        Err(e) => Err(e),
    }
}

/// Deletes a widget.
///
/// # Errors
/// Storage failure.
pub async fn delete_widget(state: &AppState, id: WidgetId) -> Result<DeleteOutcome, RepoError> {
    match state.store.widgets().delete(id).await {
        Ok(()) => Ok(DeleteOutcome::Deleted),
        Err(RepoError::NotFound) => Ok(DeleteOutcome::NotFound),
        Err(e) => Err(e),
    }
}

// ------------------------------------------------------------------ devices

/// Creates or replaces a device's parameters (its state, availability and what Zigbee2MQTT discovers do not change).
///
/// # Errors
/// Storage failure.
pub async fn save_device(
    state: &AppState,
    id: Option<DeviceId>,
    draft: &DeviceDraft,
) -> Result<SaveOutcome<Device>, RepoError> {
    let problems = draft.problems();
    if !problems.is_empty() {
        return Ok(SaveOutcome::Invalid(field_problems(problems)));
    }
    let repo = state.store.devices();
    let saved = match id {
        None => repo.create(draft).await,
        Some(id) => repo.update(id, draft).await,
    };
    match saved {
        Ok(d) => Ok(SaveOutcome::Saved(d)),
        Err(RepoError::NotFound) => Ok(SaveOutcome::NotFound),
        Err(RepoError::Conflict { constraint }) => {
            // A name already taken, or a group / place that vanished in the meantime.
            let (field, kind) = if constraint.contains("friendly_name") {
                ("friendly_name", ProblemKind::Taken)
            } else if constraint.contains("group") {
                ("group_id", ProblemKind::Invalid)
            } else {
                ("location_id", ProblemKind::Invalid)
            };
            Ok(SaveOutcome::Invalid(one(field, kind)))
        }
        Err(e) => Err(e),
    }
}

/// Deletes a device (its widgets stay, without a device; its history goes with it).
///
/// # Errors
/// Storage failure.
pub async fn delete_device(state: &AppState, id: DeviceId) -> Result<DeleteOutcome, RepoError> {
    match state.store.devices().delete(id).await {
        Ok(()) => Ok(DeleteOutcome::Deleted),
        Err(RepoError::NotFound) => Ok(DeleteOutcome::NotFound),
        Err(e) => Err(e),
    }
}

// ------------------------------------------------------------------ navigation

/// What a navigation entry needs beyond its own validation: the icon must exist in Lucide (an unknown one would show as the
/// fallback icon).
#[must_use]
pub fn check_navbar(draft: &NavbarDraft) -> Vec<FieldProblem> {
    let mut found = draft.problems();
    if found.iter().all(|(f, _)| *f != "icon_name") && !crate::icons::exists(&draft.icon_name) {
        found.push(("icon_name", dominet_domain::Problem::Invalid));
    }
    field_problems(found)
}

/// Creates or replaces a navigation entry.
///
/// # Errors
/// Storage failure.
pub async fn save_navbar(
    state: &AppState,
    id: Option<NavbarItemId>,
    draft: &NavbarDraft,
) -> Result<SaveOutcome<NavbarItem>, RepoError> {
    let problems = check_navbar(draft);
    if !problems.is_empty() {
        return Ok(SaveOutcome::Invalid(problems));
    }
    let repo = state.store.navbar();
    let saved = match id {
        None => repo.create(draft).await,
        Some(id) => repo.update(id, draft).await,
    };
    match saved {
        Ok(item) => Ok(SaveOutcome::Saved(item)),
        Err(RepoError::NotFound) => Ok(SaveOutcome::NotFound),
        Err(e) => Err(e),
    }
}

/// Deletes a navigation entry.
///
/// # Errors
/// Storage failure.
pub async fn delete_navbar(state: &AppState, id: NavbarItemId) -> Result<DeleteOutcome, RepoError> {
    match state.store.navbar().delete(id).await {
        Ok(()) => Ok(DeleteOutcome::Deleted),
        Err(RepoError::NotFound) => Ok(DeleteOutcome::NotFound),
        Err(RepoError::Conflict { .. }) => Ok(DeleteOutcome::InUse),
        Err(e) => Err(e),
    }
}

// ------------------------------------------------------------------ object detector

/// Turns the object detector and the saving of annotated frames on or off (the model and the backend are not settings here).
///
/// # Errors
/// Storage failure.
pub async fn set_detector(
    state: &AppState,
    enabled: bool,
    save_frames: bool,
) -> Result<(), RepoError> {
    let repo = state.store.detector_config();
    let mut config = repo.get().await?;
    config.enabled = enabled;
    config.save_frames = save_frames;
    repo.update(&config).await?;
    Ok(())
}

// ------------------------------------------------------------------ Rhai scripts

/// The longest script the dry run accepts, in characters.
pub const SCRIPT_MAX_CHARS: usize = 20_000;

/// What a script dry run reports: nothing was published, activated or saved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScriptRun {
    /// Syntax or run error; empty when the script ended well.
    pub error: String,
    /// What the script would have done, in order.
    pub effects: Vec<String>,
    /// Lines of `print` and `log`.
    pub logs: Vec<String>,
    pub elapsed_ms: u64,
}

/// Tries a script against a sample message, in the same sandbox as real runs and with the real device states; nothing is
/// published. Used by the interface and the MCP server.
///
/// # Errors
/// Storage failure (the devices cannot be read).
pub async fn dry_run_script(
    state: &AppState,
    code: String,
    topic: String,
    payload: &str,
) -> Result<ScriptRun, RepoError> {
    use dominet_automation::script::{self, DeviceView, Limits, ScriptInput};
    use dominet_domain::repo::DeviceFilter;

    let failed = |error: String| ScriptRun {
        error,
        effects: vec![],
        logs: vec![],
        elapsed_ms: 0,
    };
    if code.chars().count() > SCRIPT_MAX_CHARS {
        return Ok(failed("script too long".to_string()));
    }
    if let Err(e) = script::check(&code) {
        return Ok(failed(e));
    }
    let devices = state
        .store
        .devices()
        .list(&DeviceFilter {
            active_only: true,
            ..DeviceFilter::default()
        })
        .await?
        .iter()
        .map(|d| DeviceView::of(d, chrono::Utc::now()))
        .collect::<Vec<_>>();
    // The sample: JSON if it is some, else text; the device is the last level of the topic.
    let value = serde_json::from_str(payload)
        .unwrap_or_else(|_| serde_json::Value::String(payload.to_string()));
    let device = topic
        .rsplit('/')
        .next()
        .filter(|n| devices.iter().any(|d| d.name == *n))
        .map(str::to_string);
    let out = script::run_isolated(
        code,
        ScriptInput {
            topic,
            payload: value,
            device,
            devices,
        },
        Limits::default(),
    )
    .await;
    Ok(ScriptRun {
        error: out.result.err().unwrap_or_default(),
        effects: out.effects.iter().map(script::Effect::describe).collect(),
        logs: out.logs,
        elapsed_ms: u64::try_from(out.elapsed.as_millis()).unwrap_or(u64::MAX),
    })
}

/// Compiles a Rhai script without running it.
///
/// # Errors
/// The compiler's message.
pub fn compile_script(code: &str) -> Result<(), String> {
    dominet_automation::script::check(code)
}
