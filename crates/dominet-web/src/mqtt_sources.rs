// SPDX-License-Identifier: AGPL-3.0-or-later
//! Administration of MQTT sources (subscriptions outside Zigbee2MQTT): `/admin/mqtt-sources` (R3, ADR 0028).
//!
//! Any modification reloads the MQTT client's subscriptions live. The handling code (`handler_code`, reference in the
//! previous version) is kept and displayed but **is not run** before R4 (Rhai scripts).

use dominet_proto::{DeleteOutcome, MqttSourceDto, MqttSourceInput, SaveOutcome};
use leptos::prelude::*;
use leptos_fluent::move_tr;

use crate::crud::{CrudResult, CrudTable, FieldKind, FieldSpec, Labels, Row, Values};
use crate::guard::RequireAdmin;

#[cfg(feature = "ssr")]
fn dto(s: &dominet_domain::device::MqttSource) -> MqttSourceDto {
    MqttSourceDto {
        id: s.id.0,
        name: s.name.clone(),
        topic_pattern: s.topic_pattern.clone(),
        qos: s.qos,
        active: s.active,
        log_messages: s.log_messages,
        auto_device: s.auto_device,
        auto_device_type: s
            .auto_device_type
            .unwrap_or(dominet_domain::device::DeviceType::Sensor)
            .as_str()
            .to_string(),
        base_topic: s.base_topic.clone(),
        handler_code: s.handler_code.clone(),
        description: s.description.clone(),
    }
}

/// MQTT sources, by name (administrators).
///
/// # Errors
/// Refused (403) for a non-administrator; internal error if the database fails.
#[server(name = ListMqttSources, prefix = "/sf", endpoint = "mqtt-sources/list")]
pub async fn list_mqtt_sources() -> Result<Vec<MqttSourceDto>, ServerFnError> {
    use dominet_domain::repo::MqttSourceRepo;

    use crate::server_fns::{internal, require_admin};

    let state = expect_context::<dominet_server::AppState>();
    require_admin(&state).await?;
    let items = state
        .store
        .mqtt_sources()
        .list()
        .await
        .map_err(|e| internal("MQTT source list", &e))?;
    Ok(items.iter().map(dto).collect())
}

/// Creates (`id` absent) or replaces (`id` present) a source; the MQTT client's subscriptions are reloaded.
///
/// # Errors
/// Refused (403) for a non-administrator; internal error if the database fails.
#[server(name = SaveMqttSource, prefix = "/sf", endpoint = "mqtt-sources/save", input = server_fn::codec::Json)]
pub async fn save_mqtt_source(
    input: MqttSourceInput,
) -> Result<SaveOutcome<MqttSourceDto>, ServerFnError> {
    use std::str::FromStr;

    use dominet_domain::device::{DeviceType, MqttSource, MqttSourceId};

    use crate::server_fns::{internal, require_admin};

    let state = expect_context::<dominet_server::AppState>();
    let admin = require_admin(&state).await?;
    let mut draft = MqttSource::draft(input.name.trim(), input.topic_pattern.trim());
    draft.qos = input.qos;
    draft.active = input.active;
    draft.log_messages = input.log_messages;
    draft.auto_device = input.auto_device;
    draft.auto_device_type =
        Some(DeviceType::from_str(&input.auto_device_type).unwrap_or(DeviceType::Sensor));
    draft.base_topic = input.base_topic.trim().trim_end_matches('/').to_string();
    draft.handler_code = input.handler_code;
    draft.description = input.description.trim().to_string();
    let saved = dominet_server::config_service::save_mqtt_source(
        &state,
        input.id.map(MqttSourceId),
        &draft,
    )
    .await
    .map_err(|e| internal("saving an MQTT source", &e))?;
    Ok(match saved {
        SaveOutcome::Saved(s) => {
            tracing::info!(
                admin_id = admin.id.0,
                source_id = s.id.0,
                created = input.id.is_none(),
                "MQTT source saved"
            );
            SaveOutcome::Saved(dto(&s))
        }
        SaveOutcome::Invalid(p) => SaveOutcome::Invalid(p),
        SaveOutcome::NotFound => SaveOutcome::NotFound,
        SaveOutcome::Blocked(b) => SaveOutcome::Blocked(b),
    })
}

/// Deletes a source; the devices it had created stay.
///
/// # Errors
/// Refused (403) for a non-administrator; internal error if the database fails.
#[server(name = DeleteMqttSource, prefix = "/sf", endpoint = "mqtt-sources/delete")]
pub async fn delete_mqtt_source(id: i64) -> Result<DeleteOutcome, ServerFnError> {
    use dominet_domain::device::MqttSourceId;

    use crate::server_fns::{internal, require_admin};

    let state = expect_context::<dominet_server::AppState>();
    let admin = require_admin(&state).await?;
    let out = dominet_server::config_service::delete_mqtt_source(&state, MqttSourceId(id))
        .await
        .map_err(|e| internal("suppression d'une source MQTT", &e))?;
    if out == DeleteOutcome::Deleted {
        tracing::info!(admin_id = admin.id.0, source_id = id, "MQTT source deleted");
    }
    Ok(out)
}

const QOS: &[(&str, &str)] = &[
    ("0", tk!("mqtt-sources-qos-0")),
    ("1", tk!("mqtt-sources-qos-1")),
    ("2", tk!("mqtt-sources-qos-2")),
];

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

fn row(s: &MqttSourceDto) -> Row {
    Row {
        id: s.id,
        values: Values::from([
            ("name", s.name.clone()),
            ("topic_pattern", s.topic_pattern.clone()),
            ("qos", s.qos.to_string()),
            ("active", s.active.to_string()),
            ("log_messages", s.log_messages.to_string()),
            ("auto_device", s.auto_device.to_string()),
            ("auto_device_type", s.auto_device_type.clone()),
            ("base_topic", s.base_topic.clone()),
            ("handler_code", s.handler_code.clone()),
            ("description", s.description.clone()),
        ]),
    }
}

#[allow(clippy::too_many_arguments)]
fn field(
    name: &'static str,
    label: &'static str,
    kind: FieldKind,
    default: &'static str,
    required: bool,
    max_len: Option<u32>,
    hint: Option<&'static str>,
    in_table: bool,
) -> FieldSpec {
    FieldSpec {
        name,
        label,
        kind,
        default,
        required,
        max_len,
        hint,
        stored_flag: None,
        create_only: false,
        in_table,
    }
}

fn fields() -> Vec<FieldSpec> {
    let mut all = identity_fields();
    all.extend(behaviour_fields());
    all
}

fn identity_fields() -> Vec<FieldSpec> {
    vec![
        field(
            "name",
            tk!("mqtt-sources-name"),
            FieldKind::Text,
            "",
            true,
            Some(64),
            None,
            true,
        ),
        field(
            "topic_pattern",
            tk!("mqtt-sources-topic"),
            FieldKind::Text,
            "",
            true,
            Some(255),
            Some(tk!("mqtt-sources-topic-hint")),
            true,
        ),
        field(
            "qos",
            tk!("mqtt-sources-qos"),
            FieldKind::Select(QOS),
            "0",
            false,
            None,
            None,
            true,
        ),
        field(
            "active",
            tk!("mqtt-sources-active"),
            FieldKind::Checkbox,
            "true",
            false,
            None,
            None,
            true,
        ),
    ]
}

fn behaviour_fields() -> Vec<FieldSpec> {
    vec![
        field(
            "log_messages",
            tk!("mqtt-sources-log"),
            FieldKind::Checkbox,
            "false",
            false,
            None,
            Some(tk!("mqtt-sources-log-hint")),
            false,
        ),
        field(
            "auto_device",
            tk!("mqtt-sources-auto"),
            FieldKind::Checkbox,
            "false",
            false,
            None,
            Some(tk!("mqtt-sources-auto-hint")),
            true,
        ),
        field(
            "auto_device_type",
            tk!("mqtt-sources-auto-type"),
            FieldKind::Select(TYPES),
            "sensor",
            false,
            None,
            None,
            false,
        ),
        field(
            "base_topic",
            tk!("mqtt-sources-base"),
            FieldKind::Text,
            "",
            false,
            Some(128),
            Some(tk!("mqtt-sources-base-hint")),
            false,
        ),
        field(
            "handler_code",
            tk!("mqtt-sources-handler"),
            FieldKind::Code,
            "",
            false,
            Some(20_000),
            Some(tk!("mqtt-sources-handler-hint")),
            false,
        ),
        field(
            "description",
            tk!("mqtt-sources-description"),
            FieldKind::Text,
            "",
            false,
            Some(256),
            None,
            false,
        ),
    ]
}

#[component]
pub fn MqttSourcesPage() -> impl IntoView {
    view! {
        <RequireAdmin>
            <main class="vc-container">
                <MqttSourcesAdmin/>
            </main>
        </RequireAdmin>
    }
}

#[component]
fn MqttSourcesAdmin() -> impl IntoView {
    let version = RwSignal::new(0u32);
    let list = Resource::new(
        move || version.get(),
        |_| async { list_mqtt_sources().await.map_err(|_| ()) },
    );
    let rows = Signal::derive(move || match list.get() {
        Some(Ok(items)) => items.iter().map(row).collect(),
        _ => vec![],
    });

    let save = Action::new(|(id, v): &(Option<i64>, Values)| {
        let text = |k: &str| v.get(k).cloned().unwrap_or_default();
        let flag = |k: &str| v.get(k).is_some_and(|a| a == "true");
        let input = MqttSourceInput {
            id: *id,
            name: text("name"),
            topic_pattern: text("topic_pattern"),
            qos: text("qos").parse().unwrap_or(0),
            active: flag("active"),
            log_messages: flag("log_messages"),
            auto_device: flag("auto_device"),
            auto_device_type: text("auto_device_type"),
            base_topic: text("base_topic"),
            handler_code: text("handler_code"),
            description: text("description"),
        };
        async move {
            save_mqtt_source(input)
                .await
                .map_or(CrudResult::Failed, CrudResult::from)
        }
    });
    let remove = Action::new(|id: &i64| {
        let id = *id;
        async move {
            delete_mqtt_source(id)
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
                    title: tk!("mqtt-sources-title"),
                    add: tk!("mqtt-sources-add"),
                    edit: tk!("mqtt-sources-edit"),
                    empty: tk!("mqtt-sources-empty"),
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
