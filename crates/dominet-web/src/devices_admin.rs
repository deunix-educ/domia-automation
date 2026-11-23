// SPDX-License-Identifier: AGPL-3.0-or-later
//! Administration of devices, device groups and state-key labels (R3 lot 5, ADR 0029):
//! `/admin/devices`, `/admin/device-groups`, `/admin/device-labels`. The generic CRUD (ADR 0014); the drop-down lists
//! (group, location) come from the database through the [`Choices`] context.

use std::collections::BTreeMap;

use dominet_proto::{
    DeleteOutcome, DeviceAdminDto, DeviceChoicesDto, DeviceGroupDto, DeviceGroupInput, DeviceInput,
    DeviceLabelDto, DeviceLabelInput, SaveOutcome,
};
use leptos::prelude::*;
use leptos_fluent::move_tr;

use crate::crud::{Choices, CrudResult, CrudTable, FieldKind, FieldSpec, Labels, Row, Values};
use crate::guard::RequireAdmin;

// ------------------------------------------------------------------ server functions

#[cfg(feature = "ssr")]
mod ssr {
    use dominet_domain::device::{Device, DeviceGroupId};
    use dominet_domain::location::LocationId;
    use dominet_proto::{DeviceAdminDto, FieldProblem, ProblemKind};

    pub fn device_dto(d: &Device) -> DeviceAdminDto {
        DeviceAdminDto {
            id: d.id.0,
            friendly_name: d.friendly_name.clone(),
            description: d.description.clone(),
            device_type: d.device_type.as_str().to_string(),
            group_id: d.group.map(|g| g.0.to_string()).unwrap_or_default(),
            location_id: d.location.map(|l| l.0.to_string()).unwrap_or_default(),
            base_topic: d.base_topic.clone(),
            active: d.active,
            icon: d.icon.clone(),
            image_url: d.image_url.clone(),
            tracked_keys: d.tracked_keys.join(", "),
            hidden_keys: d.hidden_keys.join(", "),
            grafana_dashboard: d.grafana_dashboard,
            vendor: d.vendor.clone(),
            model_id: d.model_id.clone(),
            ieee: d.ieee.clone(),
        }
    }

    /// List of keys separated by commas (spaces and empty ones ignored).
    pub fn keys(raw: &str) -> Vec<String> {
        raw.split(',')
            .map(str::trim)
            .filter(|k| !k.is_empty())
            .map(str::to_string)
            .collect()
    }

    /// Optional number from a drop-down list: empty = none; anything other than a positive integer = invalid field.
    pub fn optional_id(raw: &str, field: &str) -> Result<Option<i64>, FieldProblem> {
        let raw = raw.trim();
        if raw.is_empty() {
            return Ok(None);
        }
        raw.parse::<i64>()
            .ok()
            .filter(|n| *n > 0)
            .map(Some)
            .ok_or_else(|| FieldProblem {
                field: field.to_string(),
                kind: ProblemKind::Invalid,
            })
    }

    pub fn group(id: Option<i64>) -> Option<DeviceGroupId> {
        id.map(DeviceGroupId)
    }

    pub fn location(id: Option<i64>) -> Option<LocationId> {
        id.map(LocationId)
    }
}

/// Devices, by name (administrators).
///
/// # Errors
/// Refused (403) for a non-administrator; internal error if the database fails.
#[server(name = ListDevicesAdmin, prefix = "/sf", endpoint = "devices-admin/list")]
pub async fn list_devices_admin() -> Result<Vec<DeviceAdminDto>, ServerFnError> {
    use dominet_domain::repo::{DeviceFilter, DeviceRepo};

    use crate::server_fns::{internal, require_admin};

    let state = expect_context::<dominet_server::AppState>();
    require_admin(&state).await?;
    let items = state
        .store
        .devices()
        .list(&DeviceFilter::default())
        .await
        .map_err(|e| internal("device list", &e))?;
    Ok(items.iter().map(ssr::device_dto).collect())
}

/// Creates (`id` absent) or modifies a device. The state, the availability and what Zigbee2MQTT discovers do not change.
///
/// # Errors
/// Refused (403) for a non-administrator; internal error if the database fails.
#[server(name = SaveDeviceAdmin, prefix = "/sf", endpoint = "devices-admin/save", input = server_fn::codec::Json)]
pub async fn save_device_admin(
    input: DeviceInput,
) -> Result<SaveOutcome<DeviceAdminDto>, ServerFnError> {
    use std::str::FromStr;

    use dominet_domain::device::{DeviceDraft, DeviceId, DeviceType};

    use crate::server_fns::{field_problems, internal, require_admin};

    let state = expect_context::<dominet_server::AppState>();
    let admin = require_admin(&state).await?;
    let mut bad = vec![];
    let group = ssr::optional_id(&input.group_id, "group_id").unwrap_or_else(|p| {
        bad.push(p);
        None
    });
    let location = ssr::optional_id(&input.location_id, "location_id").unwrap_or_else(|p| {
        bad.push(p);
        None
    });
    let Ok(device_type) = DeviceType::from_str(&input.device_type) else {
        bad.push(dominet_proto::FieldProblem {
            field: "device_type".into(),
            kind: dominet_proto::ProblemKind::Invalid,
        });
        return Ok(SaveOutcome::Invalid(bad));
    };
    let draft = DeviceDraft {
        friendly_name: input.friendly_name.trim().to_string(),
        description: input.description.trim().to_string(),
        device_type,
        group: ssr::group(group),
        location: ssr::location(location),
        base_topic: input.base_topic.trim().trim_end_matches('/').to_string(),
        active: input.active,
        icon: input.icon.trim().to_string(),
        image_url: input.image_url.trim().to_string(),
        tracked_keys: ssr::keys(&input.tracked_keys),
        hidden_keys: ssr::keys(&input.hidden_keys),
        grafana_dashboard: input.grafana_dashboard,
    };
    bad.extend(field_problems(draft.problems()));
    if !bad.is_empty() {
        return Ok(SaveOutcome::Invalid(bad));
    }
    match dominet_server::config_service::save_device(&state, input.id.map(DeviceId), &draft).await
    {
        Ok(SaveOutcome::Saved(d)) => {
            tracing::info!(
                admin_id = admin.id.0,
                device_id = d.id.0,
                created = input.id.is_none(),
                "device saved"
            );
            Ok(SaveOutcome::Saved(ssr::device_dto(&d)))
        }
        Ok(other) => Ok(other.map(|d| ssr::device_dto(&d))),
        Err(e) => Err(internal("saving a device", &e)),
    }
}

/// Deletes a device (its widgets stay, without a device; its history is deleted with it).
///
/// # Errors
/// Refused (403) for a non-administrator; internal error if the database fails.
#[server(name = DeleteDeviceAdmin, prefix = "/sf", endpoint = "devices-admin/delete")]
pub async fn delete_device_admin(id: i64) -> Result<DeleteOutcome, ServerFnError> {
    use dominet_domain::device::DeviceId;

    use crate::server_fns::{internal, require_admin};

    let state = expect_context::<dominet_server::AppState>();
    let admin = require_admin(&state).await?;
    match dominet_server::config_service::delete_device(&state, DeviceId(id)).await {
        Ok(outcome) => {
            if matches!(outcome, DeleteOutcome::Deleted) {
                tracing::info!(admin_id = admin.id.0, device_id = id, "device deleted");
            }
            Ok(outcome)
        }
        Err(e) => Err(internal("deleting a device", &e)),
    }
}

/// Groups, locations and devices for the drop-down lists.
///
/// # Errors
/// Refused (403) for a non-administrator; internal error if the database fails.
#[server(name = DeviceChoices, prefix = "/sf", endpoint = "devices-admin/choices")]
pub async fn device_choices() -> Result<DeviceChoicesDto, ServerFnError> {
    use dominet_domain::repo::{
        DeviceFilter, DeviceGroupRepo, DeviceRepo, LocationRepo, SceneRepo,
    };

    use crate::server_fns::{internal, require_admin};

    let state = expect_context::<dominet_server::AppState>();
    require_admin(&state).await?;
    let groups = state
        .store
        .device_groups()
        .list()
        .await
        .map_err(|e| internal("group list", &e))?;
    let locations = state
        .store
        .locations()
        .list()
        .await
        .map_err(|e| internal("place list", &e))?;
    let devices = state
        .store
        .devices()
        .list(&DeviceFilter::default())
        .await
        .map_err(|e| internal("device list", &e))?;
    let scenes = state
        .store
        .scenes()
        .list()
        .await
        .map_err(|e| internal("scene list", &e))?;
    Ok(DeviceChoicesDto {
        scenes: scenes.into_iter().map(|s| (s.id.0, s.name)).collect(),
        groups: groups.into_iter().map(|g| (g.id.0, g.name)).collect(),
        locations: locations
            .into_iter()
            .filter(|l| l.active)
            .map(|l| (l.id.0, l.name))
            .collect(),
        devices: devices
            .into_iter()
            .map(|d| (d.id.0, d.friendly_name))
            .collect(),
    })
}

/// Device groups, by name (administrators).
///
/// # Errors
/// Refused (403) for a non-administrator; internal error if the database fails.
#[server(name = ListDeviceGroups, prefix = "/sf", endpoint = "device-groups/list")]
pub async fn list_device_groups() -> Result<Vec<DeviceGroupDto>, ServerFnError> {
    use dominet_domain::repo::DeviceGroupRepo;

    use crate::server_fns::{internal, require_admin};

    let state = expect_context::<dominet_server::AppState>();
    require_admin(&state).await?;
    let items = state
        .store
        .device_groups()
        .list()
        .await
        .map_err(|e| internal("group list", &e))?;
    Ok(items
        .into_iter()
        .map(|g| DeviceGroupDto {
            id: g.id.0,
            name: g.name,
            location_id: g.location.map(|l| l.0.to_string()).unwrap_or_default(),
            description: g.description,
            icon: g.icon,
        })
        .collect())
}

/// Creates or modifies a device group.
///
/// # Errors
/// Refused (403) for a non-administrator; internal error if the database fails.
#[server(name = SaveDeviceGroup, prefix = "/sf", endpoint = "device-groups/save", input = server_fn::codec::Json)]
pub async fn save_device_group(
    input: DeviceGroupInput,
) -> Result<SaveOutcome<DeviceGroupDto>, ServerFnError> {
    use dominet_domain::device::{DeviceGroupDraft, DeviceGroupId};

    use crate::server_fns::{internal, require_admin};

    let state = expect_context::<dominet_server::AppState>();
    let admin = require_admin(&state).await?;
    let location = match ssr::optional_id(&input.location_id, "location_id") {
        Ok(l) => l,
        Err(p) => return Ok(SaveOutcome::Invalid(vec![p])),
    };
    let draft = DeviceGroupDraft {
        name: input.name.trim().to_string(),
        location: ssr::location(location),
        description: input.description.trim().to_string(),
        icon: input.icon.trim().to_string(),
    };
    let saved = dominet_server::config_service::save_device_group(
        &state,
        input.id.map(DeviceGroupId),
        &draft,
    )
    .await
    .map_err(|e| internal("saving a group", &e))?;
    Ok(match saved {
        SaveOutcome::Saved(g) => {
            tracing::info!(
                admin_id = admin.id.0,
                group_id = g.id.0,
                created = input.id.is_none(),
                "device group saved"
            );
            SaveOutcome::Saved(DeviceGroupDto {
                id: g.id.0,
                name: g.name,
                location_id: g.location.map(|l| l.0.to_string()).unwrap_or_default(),
                description: g.description,
                icon: g.icon,
            })
        }
        SaveOutcome::Invalid(p) => SaveOutcome::Invalid(p),
        SaveOutcome::NotFound => SaveOutcome::NotFound,
        SaveOutcome::Blocked(b) => SaveOutcome::Blocked(b),
    })
}

/// Deletes a group: its devices stay, without a group.
///
/// # Errors
/// Refused (403) for a non-administrator; internal error if the database fails.
#[server(name = DeleteDeviceGroup, prefix = "/sf", endpoint = "device-groups/delete")]
pub async fn delete_device_group(id: i64) -> Result<DeleteOutcome, ServerFnError> {
    use dominet_domain::device::DeviceGroupId;

    use crate::server_fns::{internal, require_admin};

    let state = expect_context::<dominet_server::AppState>();
    let admin = require_admin(&state).await?;
    let out = dominet_server::config_service::delete_device_group(&state, DeviceGroupId(id))
        .await
        .map_err(|e| internal("deleting a group", &e))?;
    if out == DeleteOutcome::Deleted {
        tracing::info!(admin_id = admin.id.0, group_id = id, "device group deleted");
    }
    Ok(out)
}

/// State-key labels, by key (administrators).
///
/// # Errors
/// Refused (403) for a non-administrator; internal error if the database fails.
#[server(name = ListDeviceLabels, prefix = "/sf", endpoint = "device-labels/list")]
pub async fn list_device_labels() -> Result<Vec<DeviceLabelDto>, ServerFnError> {
    use dominet_domain::repo::DeviceLabelRepo;

    use crate::server_fns::{internal, require_admin};

    let state = expect_context::<dominet_server::AppState>();
    require_admin(&state).await?;
    let items = state
        .store
        .device_labels()
        .list()
        .await
        .map_err(|e| internal("label list", &e))?;
    Ok(items
        .into_iter()
        .map(|l| DeviceLabelDto {
            id: l.id.0,
            key: l.key,
            name: l.name,
            icon_name: l.icon_name,
            unit: l.unit,
        })
        .collect())
}

/// Creates or modifies a state key's label.
///
/// # Errors
/// Refused (403) for a non-administrator; internal error if the database fails.
#[server(name = SaveDeviceLabel, prefix = "/sf", endpoint = "device-labels/save", input = server_fn::codec::Json)]
pub async fn save_device_label(
    input: DeviceLabelInput,
) -> Result<SaveOutcome<DeviceLabelDto>, ServerFnError> {
    use dominet_domain::device::{DeviceLabelDraft, DeviceLabelId};

    use crate::server_fns::{internal, require_admin};

    let state = expect_context::<dominet_server::AppState>();
    let admin = require_admin(&state).await?;
    let draft = DeviceLabelDraft {
        key: input.key.trim().to_string(),
        name: input.name.trim().to_string(),
        icon_name: input.icon_name.trim().to_string(),
        unit: input.unit.trim().to_string(),
    };
    let saved = dominet_server::config_service::save_device_label(
        &state,
        input.id.map(DeviceLabelId),
        &draft,
    )
    .await
    .map_err(|e| internal("saving a label", &e))?;
    Ok(match saved {
        SaveOutcome::Saved(l) => {
            tracing::info!(
                admin_id = admin.id.0,
                label_id = l.id.0,
                created = input.id.is_none(),
                "label saved"
            );
            SaveOutcome::Saved(DeviceLabelDto {
                id: l.id.0,
                key: l.key,
                name: l.name,
                icon_name: l.icon_name,
                unit: l.unit,
            })
        }
        SaveOutcome::Invalid(p) => SaveOutcome::Invalid(p),
        SaveOutcome::NotFound => SaveOutcome::NotFound,
        SaveOutcome::Blocked(b) => SaveOutcome::Blocked(b),
    })
}

/// Deletes a key's label.
///
/// # Errors
/// Refused (403) for a non-administrator; internal error if the database fails.
#[server(name = DeleteDeviceLabel, prefix = "/sf", endpoint = "device-labels/delete")]
pub async fn delete_device_label(id: i64) -> Result<DeleteOutcome, ServerFnError> {
    use dominet_domain::device::DeviceLabelId;

    use crate::server_fns::{internal, require_admin};

    let state = expect_context::<dominet_server::AppState>();
    let admin = require_admin(&state).await?;
    let out = dominet_server::config_service::delete_device_label(&state, DeviceLabelId(id))
        .await
        .map_err(|e| internal("deleting a label", &e))?;
    if out == DeleteOutcome::Deleted {
        tracing::info!(admin_id = admin.id.0, label_id = id, "label deleted");
    }
    Ok(out)
}

// ------------------------------------------------------------------ pages

const TYPES: &[(&str, &str)] = &[
    ("sensor", tk!("devtype-sensor")),
    ("binary_sensor", tk!("devtype-binary-sensor")),
    ("switch", tk!("devtype-switch")),
    ("light", tk!("devtype-light")),
    ("cover", tk!("devtype-cover")),
    ("climate", tk!("devtype-climate")),
    ("remote", tk!("devtype-remote")),
    ("generic", tk!("devtype-generic")),
];

fn text(v: &Values, key: &str) -> String {
    v.get(key).cloned().unwrap_or_default()
}

fn flag(v: &Values, key: &str) -> bool {
    v.get(key).is_some_and(|a| a == "true")
}

/// Drop-down list choices, loaded once per page.
pub(crate) fn provide_choices(wanted: &'static [&'static str]) {
    let loaded = Resource::new(
        || (),
        |()| async { device_choices().await.unwrap_or_default() },
    );
    let map = Signal::derive(move || {
        let c = loaded.get().unwrap_or_default();
        let list = |v: Vec<(i64, String)>| {
            v.into_iter()
                .map(|(id, n)| (id.to_string(), n))
                .collect::<Vec<_>>()
        };
        let mut m = BTreeMap::new();
        for key in wanted {
            match *key {
                "group_id" => m.insert("group_id", list(c.groups.clone())),
                "location_id" => m.insert("location_id", list(c.locations.clone())),
                "actions" => m.insert("actions", list(c.devices.clone())),
                "scene_id" => m.insert("scene_id", list(c.scenes.clone())),
                "device_name" => m.insert(
                    "friendly_name",
                    c.devices
                        .iter()
                        .map(|(_, n)| (n.clone(), n.clone()))
                        .collect(),
                ),
                _ => m.insert("device_id", list(c.devices.clone())),
            };
        }
        m
    });
    provide_context(Choices(map));
}

fn device_row(d: &DeviceAdminDto) -> Row {
    Row {
        id: d.id,
        values: Values::from([
            ("friendly_name", d.friendly_name.clone()),
            ("description", d.description.clone()),
            ("device_type", d.device_type.clone()),
            ("group_id", d.group_id.clone()),
            ("location_id", d.location_id.clone()),
            ("base_topic", d.base_topic.clone()),
            ("active", d.active.to_string()),
            ("icon", d.icon.clone()),
            ("image_url", d.image_url.clone()),
            ("tracked_keys", d.tracked_keys.clone()),
            ("hidden_keys", d.hidden_keys.clone()),
            ("grafana_dashboard", d.grafana_dashboard.to_string()),
        ]),
    }
}

fn device_fields() -> Vec<FieldSpec> {
    use FieldKind as K;
    vec![
        FieldSpec::new("friendly_name", tk!("devices-name"), K::Text)
            .required()
            .max(128)
            .column(),
        FieldSpec::new("device_type", tk!("devices-type"), K::Select(TYPES))
            .default_value("generic")
            .column(),
        FieldSpec::new("group_id", tk!("devices-group"), K::Choice).column(),
        FieldSpec::new("location_id", tk!("devices-location"), K::Choice).column(),
        FieldSpec::new("active", tk!("devices-active"), K::Checkbox)
            .default_value("true")
            .column(),
        FieldSpec::new("description", tk!("devices-description"), K::Text).max(256),
        FieldSpec::new("base_topic", tk!("devices-base-topic"), K::Text)
            .default_value("zigbee2mqtt")
            .max(128)
            .hint(tk!("devices-base-topic-hint")),
        FieldSpec::new("icon", tk!("devices-icon"), K::Text).max(32),
        FieldSpec::new("image_url", tk!("devices-image-url"), K::Text).max(512),
        FieldSpec::new("tracked_keys", tk!("devices-tracked"), K::Text)
            .hint(tk!("devices-tracked-hint")),
        FieldSpec::new("hidden_keys", tk!("devices-hidden"), K::Text)
            .default_value("update")
            .hint(tk!("devices-hidden-hint")),
        FieldSpec::new("grafana_dashboard", tk!("devices-grafana"), K::Checkbox)
            .hint(tk!("devices-grafana-hint")),
    ]
}

#[component]
pub fn DevicesAdminPage() -> impl IntoView {
    view! {
        <RequireAdmin>
            <main class="vc-container">
                <DevicesAdmin/>
            </main>
        </RequireAdmin>
    }
}

#[component]
fn DevicesAdmin() -> impl IntoView {
    provide_choices(&["group_id", "location_id"]);
    let version = RwSignal::new(0u32);
    let list = Resource::new(
        move || version.get(),
        |_| async { list_devices_admin().await.map_err(|_| ()) },
    );
    let rows = Signal::derive(move || match list.get() {
        Some(Ok(items)) => items.iter().map(device_row).collect(),
        _ => vec![],
    });
    let save = Action::new(|(id, v): &(Option<i64>, Values)| {
        let input = DeviceInput {
            id: *id,
            friendly_name: text(v, "friendly_name"),
            description: text(v, "description"),
            device_type: text(v, "device_type"),
            group_id: text(v, "group_id"),
            location_id: text(v, "location_id"),
            base_topic: text(v, "base_topic"),
            active: flag(v, "active"),
            icon: text(v, "icon"),
            image_url: text(v, "image_url"),
            tracked_keys: text(v, "tracked_keys"),
            hidden_keys: text(v, "hidden_keys"),
            grafana_dashboard: flag(v, "grafana_dashboard"),
        };
        async move {
            save_device_admin(input)
                .await
                .map_or(CrudResult::Failed, CrudResult::from)
        }
    });
    let remove = Action::new(|id: &i64| {
        let id = *id;
        async move {
            delete_device_admin(id)
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
                    title: tk!("devices-title"),
                    add: tk!("devices-add"),
                    edit: tk!("devices-edit"),
                    empty: tk!("devices-empty"),
                }
                fields=device_fields()
                rows
                save
                remove
                on_change=Callback::new(move |()| version.update(|v| *v += 1))
            />
        </Transition>
    }
}

#[component]
pub fn DeviceGroupsPage() -> impl IntoView {
    view! {
        <RequireAdmin>
            <main class="vc-container">
                <DeviceGroupsAdmin/>
            </main>
        </RequireAdmin>
    }
}

#[component]
fn DeviceGroupsAdmin() -> impl IntoView {
    provide_choices(&["location_id"]);
    let version = RwSignal::new(0u32);
    let list = Resource::new(
        move || version.get(),
        |_| async { list_device_groups().await.map_err(|_| ()) },
    );
    let rows = Signal::derive(move || match list.get() {
        Some(Ok(items)) => items
            .iter()
            .map(|g| Row {
                id: g.id,
                values: Values::from([
                    ("name", g.name.clone()),
                    ("location_id", g.location_id.clone()),
                    ("description", g.description.clone()),
                    ("icon", g.icon.clone()),
                ]),
            })
            .collect(),
        _ => vec![],
    });
    let save = Action::new(|(id, v): &(Option<i64>, Values)| {
        let input = DeviceGroupInput {
            id: *id,
            name: text(v, "name"),
            location_id: text(v, "location_id"),
            description: text(v, "description"),
            icon: text(v, "icon"),
        };
        async move {
            save_device_group(input)
                .await
                .map_or(CrudResult::Failed, CrudResult::from)
        }
    });
    let remove = Action::new(|id: &i64| {
        let id = *id;
        async move {
            delete_device_group(id)
                .await
                .map_or(CrudResult::Failed, CrudResult::from)
        }
    });
    let fields = vec![
        FieldSpec::new("name", tk!("groups-name"), FieldKind::Text)
            .required()
            .max(64)
            .column(),
        FieldSpec::new("location_id", tk!("devices-location"), FieldKind::Choice).column(),
        FieldSpec::new("description", tk!("devices-description"), FieldKind::Text)
            .max(256)
            .column(),
        FieldSpec::new("icon", tk!("devices-icon"), FieldKind::Text).max(32),
    ];
    view! {
        <Transition fallback=|| ()>
            {move || {
                matches!(list.get(), Some(Err(()))).then(|| {
                    view! { <div class="vc-alert vc-alert--error" role="alert" id="crud-load-error">{move_tr!("crud-load-error")}</div> }
                })
            }}
            <CrudTable
                labels=Labels {
                    title: tk!("groups-title"),
                    add: tk!("groups-add"),
                    edit: tk!("groups-edit"),
                    empty: tk!("groups-empty"),
                }
                fields
                rows
                save
                remove
                on_change=Callback::new(move |()| version.update(|v| *v += 1))
            />
        </Transition>
    }
}

#[component]
pub fn DeviceLabelsPage() -> impl IntoView {
    view! {
        <RequireAdmin>
            <main class="vc-container">
                <DeviceLabelsAdmin/>
            </main>
        </RequireAdmin>
    }
}

#[component]
fn DeviceLabelsAdmin() -> impl IntoView {
    let version = RwSignal::new(0u32);
    let list = Resource::new(
        move || version.get(),
        |_| async { list_device_labels().await.map_err(|_| ()) },
    );
    let rows = Signal::derive(move || match list.get() {
        Some(Ok(items)) => items
            .iter()
            .map(|l| Row {
                id: l.id,
                values: Values::from([
                    ("key", l.key.clone()),
                    ("name", l.name.clone()),
                    ("icon_name", l.icon_name.clone()),
                    ("unit", l.unit.clone()),
                ]),
            })
            .collect(),
        _ => vec![],
    });
    let save = Action::new(|(id, v): &(Option<i64>, Values)| {
        let input = DeviceLabelInput {
            id: *id,
            key: text(v, "key"),
            name: text(v, "name"),
            icon_name: text(v, "icon_name"),
            unit: text(v, "unit"),
        };
        async move {
            save_device_label(input)
                .await
                .map_or(CrudResult::Failed, CrudResult::from)
        }
    });
    let remove = Action::new(|id: &i64| {
        let id = *id;
        async move {
            delete_device_label(id)
                .await
                .map_or(CrudResult::Failed, CrudResult::from)
        }
    });
    let fields = vec![
        FieldSpec::new("key", tk!("labels-key"), FieldKind::Text)
            .required()
            .max(64)
            .column()
            .hint(tk!("labels-key-hint")),
        FieldSpec::new("name", tk!("labels-name"), FieldKind::Text)
            .required()
            .max(128)
            .column(),
        FieldSpec::new("icon_name", tk!("labels-icon"), FieldKind::Text)
            .required()
            .default_value("activity")
            .max(64)
            .column(),
        FieldSpec::new("unit", tk!("labels-unit"), FieldKind::Text)
            .max(32)
            .column(),
    ];
    view! {
        <Transition fallback=|| ()>
            {move || {
                matches!(list.get(), Some(Err(()))).then(|| {
                    view! { <div class="vc-alert vc-alert--error" role="alert" id="crud-load-error">{move_tr!("crud-load-error")}</div> }
                })
            }}
            <CrudTable
                labels=Labels {
                    title: tk!("labels-title"),
                    add: tk!("labels-add"),
                    edit: tk!("labels-edit"),
                    empty: tk!("labels-empty"),
                }
                fields
                rows
                save
                remove
                on_change=Callback::new(move |()| version.update(|v| *v += 1))
            />
        </Transition>
    }
}
