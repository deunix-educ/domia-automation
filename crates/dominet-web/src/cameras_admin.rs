// SPDX-License-Identifier: AGPL-3.0-or-later
//! Cameras administration: `/admin/cameras` (R6 lot 2, ADR 0044).
//!
//! Passwords are never prefilled or sent back: empty field = unchanged. A button synchronises MediaMTX with the database.

use std::collections::BTreeMap;

use dominet_proto::{CameraDto, CameraInput, MediaMtxSyncDto};
use leptos::prelude::*;
use leptos_fluent::move_tr;

use crate::camera_fns::{delete_camera, list_cameras, save_camera, sync_cameras};
use crate::crud::{Choices, CrudResult, CrudTable, FieldKind, FieldSpec, Labels, Row, Values};
use crate::guard::RequireAdmin;
use crate::server_fns::list_locations;

const PROTOS: &[(&str, &str)] = &[
    ("rtsp", tk!("cameras-proto-rtsp")),
    ("rtmp", tk!("cameras-proto-rtmp")),
];
const RECORDING_MODES: &[(&str, &str)] = &[
    ("none", tk!("cameras-rec-none")),
    ("continuous", tk!("cameras-rec-continuous")),
    ("motion", tk!("cameras-rec-motion")),
    ("manual", tk!("cameras-rec-manual")),
];
const NIGHT_APIS: &[(&str, &str)] = &[
    ("none", tk!("cameras-night-none")),
    ("onvif", tk!("cameras-night-onvif")),
    ("openipc", tk!("cameras-night-openipc")),
    ("thingino", tk!("cameras-night-thingino")),
    ("goform", tk!("cameras-night-goform")),
];

/// Presence of a secret, without showing it.
fn mark(set: bool) -> &'static str {
    if set { "✔" } else { "✘" }
}

fn row(c: &CameraDto) -> Row {
    let b = |v: bool| v.to_string();
    Row {
        id: c.id,
        values: Values::from([
            ("name", c.name.clone()),
            ("uuid", c.uuid.clone()),
            (
                "location_id",
                c.location_id.map(|i| i.to_string()).unwrap_or_default(),
            ),
            ("active", b(c.active)),
            ("priority", c.priority.to_string()),
            ("notes", c.notes.clone()),
            ("proto", c.proto.clone()),
            ("ip", c.ip.clone()),
            ("port", c.port.to_string()),
            ("path", c.path.clone()),
            ("rtsp_user", c.rtsp_user.clone()),
            ("fps", c.fps.to_string()),
            ("rtsp_any_port", b(c.rtsp_any_port)),
            ("rtsp_transport_tcp", b(c.rtsp_transport_tcp)),
            ("recording_mode", c.recording_mode.clone()),
            ("night_mode_api", c.night_mode_api.clone()),
            ("web_port", c.web_port.to_string()),
            ("web_user", c.web_user.clone()),
            ("onvif", b(c.onvif)),
            ("onvif_port", c.onvif_port.to_string()),
            ("onvif_user", c.onvif_user.clone()),
            ("ptz", b(c.ptz)),
            ("main_token", c.main_token.clone()),
            (
                "last_seen",
                c.last_seen
                    .as_deref()
                    .map(|t| t.chars().take(19).collect::<String>().replace('T', " "))
                    .unwrap_or_default(),
            ),
            ("rtsp_pass_set", b(c.has_rtsp_pass)),
            ("web_api_key_set", b(c.has_web_api_key)),
            ("web_pass_set", b(c.has_web_pass)),
            ("onvif_pass_set", b(c.has_onvif_pass)),
            (
                "secrets_state",
                format!(
                    "RTSP {} · Web {} · API {} · ONVIF {}",
                    mark(c.has_rtsp_pass),
                    mark(c.has_web_pass),
                    mark(c.has_web_api_key),
                    mark(c.has_onvif_pass)
                ),
            ),
            ("yolo_enabled", b(c.yolo_enabled)),
            ("yolo_classes", c.yolo_classes.clone()),
            (
                "yolo_min_confidence_pct",
                c.yolo_min_confidence_pct.to_string(),
            ),
            (
                "device_info",
                [
                    c.manufacturer.as_str(),
                    c.model.as_str(),
                    c.firmware.as_str(),
                ]
                .iter()
                .filter(|s| !s.is_empty())
                .copied()
                .collect::<Vec<_>>()
                .join(" \u{b7} "),
            ),
        ]),
    }
}

fn number(
    name: &'static str,
    label: &'static str,
    min: i64,
    max: i64,
    default: &'static str,
) -> FieldSpec {
    FieldSpec::new(name, label, FieldKind::Number { min, max }).default_value(default)
}

fn fields() -> Vec<FieldSpec> {
    use FieldKind as K;
    vec![
        FieldSpec::new("name", tk!("cameras-name"), K::Text)
            .required()
            .max(128)
            .column(),
        FieldSpec::new("uuid", tk!("cameras-uuid"), K::Display).column(),
        FieldSpec::new("location_id", tk!("cameras-location"), K::Choice),
        number("priority", tk!("cameras-priority"), 1, 9, "1")
            .hint(tk!("cameras-priority-hint"))
            .column(),
        FieldSpec::new("active", tk!("cameras-active"), K::Checkbox)
            .default_value("true")
            .column(),
        FieldSpec::new("notes", tk!("cameras-notes"), K::TextArea),
        FieldSpec::new("proto", tk!("cameras-proto"), K::Select(PROTOS)).default_value("rtsp"),
        FieldSpec::new("ip", tk!("cameras-ip"), K::Text)
            .required()
            .max(64)
            .column(),
        number("port", tk!("cameras-port"), 1, 65535, "554"),
        FieldSpec::new("path", tk!("cameras-path"), K::Text).max(128),
        FieldSpec::new("rtsp_user", tk!("cameras-rtsp-user"), K::Text).max(128),
        FieldSpec::new("rtsp_pass", tk!("cameras-rtsp-pass"), K::Password)
            .stored_flag("rtsp_pass_set")
            .hint(tk!("cameras-secret-hint")),
        number("fps", tk!("cameras-fps"), 1, 60, "10"),
        FieldSpec::new(
            "recording_mode",
            tk!("cameras-recording-mode"),
            K::Select(RECORDING_MODES),
        )
        .default_value("none")
        .column(),
        FieldSpec::new("rtsp_any_port", tk!("cameras-any-port"), K::Checkbox)
            .hint(tk!("cameras-any-port-hint")),
        FieldSpec::new("rtsp_transport_tcp", tk!("cameras-tcp"), K::Checkbox)
            .hint(tk!("cameras-tcp-hint")),
        FieldSpec::new(
            "night_mode_api",
            tk!("cameras-night"),
            K::Select(NIGHT_APIS),
        )
        .default_value("none"),
        number("web_port", tk!("cameras-web-port"), 1, 65535, "80"),
        FieldSpec::new("web_api_key", tk!("cameras-web-key"), K::Password)
            .stored_flag("web_api_key_set")
            .hint(tk!("cameras-secret-hint")),
        FieldSpec::new("web_user", tk!("cameras-web-user"), K::Text).max(128),
        FieldSpec::new("web_pass", tk!("cameras-web-pass"), K::Password)
            .stored_flag("web_pass_set")
            .hint(tk!("cameras-secret-hint")),
        FieldSpec::new("onvif", tk!("cameras-onvif"), K::Checkbox).hint(tk!("cameras-onvif-hint")),
        number("onvif_port", tk!("cameras-onvif-port"), 1, 65535, "80"),
        FieldSpec::new("onvif_user", tk!("cameras-onvif-user"), K::Text).max(128),
        FieldSpec::new("onvif_pass", tk!("cameras-onvif-pass"), K::Password)
            .stored_flag("onvif_pass_set")
            .hint(tk!("cameras-secret-hint")),
        FieldSpec::new("main_token", tk!("cameras-main-token"), K::Text)
            .max(64)
            .hint(tk!("cameras-main-token-hint")),
        FieldSpec::new("ptz", tk!("cameras-ptz"), K::Checkbox).hint(tk!("cameras-ptz-hint")),
        FieldSpec::new("secrets_state", tk!("cameras-secrets-state"), K::Display).column(),
        FieldSpec::new("yolo_enabled", tk!("cameras-yolo"), K::Checkbox),
        FieldSpec::new("yolo_classes", tk!("cameras-yolo-classes"), K::Text)
            .hint(tk!("cameras-yolo-classes-hint")),
        number(
            "yolo_min_confidence_pct",
            tk!("cameras-yolo-confidence"),
            0,
            100,
            "50",
        ),
        FieldSpec::new("device_info", tk!("cameras-device-info"), K::Display).column(),
        FieldSpec::new("last_seen", tk!("cameras-last-seen"), K::Display),
    ]
}

#[component]
pub fn CamerasAdminPage() -> impl IntoView {
    view! {
        <RequireAdmin>
            <main class="vc-container">
                <CamerasAdmin/>
            </main>
        </RequireAdmin>
    }
}

fn input_from(id: Option<i64>, v: &Values) -> CameraInput {
    let get = |k: &str| v.get(k).cloned().unwrap_or_default();
    let int = |k: &str, default: i32| get(k).trim().parse().unwrap_or(default);
    let flag = |k: &str| get(k) == "true";
    CameraInput {
        id,
        name: get("name"),
        location_id: get("location_id").parse().ok(),
        active: flag("active"),
        priority: int("priority", 1),
        notes: get("notes"),
        proto: get("proto"),
        ip: get("ip"),
        port: int("port", 554),
        path: get("path"),
        rtsp_user: get("rtsp_user"),
        rtsp_pass: get("rtsp_pass"),
        fps: int("fps", 10),
        rtsp_any_port: flag("rtsp_any_port"),
        rtsp_transport_tcp: flag("rtsp_transport_tcp"),
        recording_mode: get("recording_mode"),
        night_mode_api: get("night_mode_api"),
        web_port: int("web_port", 80),
        web_api_key: get("web_api_key"),
        web_user: get("web_user"),
        web_pass: get("web_pass"),
        onvif: flag("onvif"),
        onvif_port: int("onvif_port", 80),
        onvif_user: get("onvif_user"),
        onvif_pass: get("onvif_pass"),
        ptz: flag("ptz"),
        yolo_enabled: flag("yolo_enabled"),
        yolo_classes: get("yolo_classes"),
        yolo_min_confidence_pct: int("yolo_min_confidence_pct", 50),
        main_token: Some(get("main_token")),
    }
}

#[component]
fn CamerasAdmin() -> impl IntoView {
    let version = RwSignal::new(0u32);
    let list = Resource::new(
        move || version.get(),
        |_| async { list_cameras().await.map_err(|_| ()) },
    );
    let locations = Resource::new(
        || (),
        |()| async { list_locations().await.unwrap_or_default() },
    );
    provide_context(Choices(Signal::derive(move || {
        let places = locations
            .get()
            .unwrap_or_default()
            .into_iter()
            .filter(|l| l.active)
            .map(|l| (l.id.to_string(), l.name))
            .collect();
        BTreeMap::from([("location_id", places)])
    })));
    let rows = Signal::derive(move || match list.get() {
        Some(Ok(items)) => items.iter().map(row).collect(),
        _ => vec![],
    });

    let save = Action::new(|(id, v): &(Option<i64>, Values)| {
        let input = input_from(*id, v);
        async move {
            save_camera(input)
                .await
                .map_or(CrudResult::Failed, CrudResult::from)
        }
    });
    let remove = Action::new(|id: &i64| {
        let id = *id;
        async move {
            delete_camera(id)
                .await
                .map_or(CrudResult::Failed, CrudResult::from)
        }
    });
    let sync = Action::new(|(): &()| async { sync_cameras().await.ok() });
    let sync_message = move || {
        sync.value().get().map(|r| match r {
            None => view! { <div class="vc-alert vc-alert--error" role="alert" id="cameras-sync-result">{move_tr!("crud-load-error")}</div> }.into_any(),
            Some(MediaMtxSyncDto::Disabled) => view! { <div class="vc-alert vc-alert--info" id="cameras-sync-result">{move_tr!("cameras-sync-disabled")}</div> }.into_any(),
            Some(MediaMtxSyncDto::Unreachable(e)) => view! { <div class="vc-alert vc-alert--error" role="alert" id="cameras-sync-result">{move_tr!("cameras-sync-unreachable")}" "{e}</div> }.into_any(),
            Some(MediaMtxSyncDto::Done { added, replaced, removed, failed }) => {
                let failures = failed
                    .iter()
                    .map(|(cam, msg)| format!("{cam} : {msg}"))
                    .collect::<Vec<_>>()
                    .join(" ; ");
                let class = if failed.is_empty() { "vc-alert vc-alert--success" } else { "vc-alert vc-alert--error" };
                view! {
                    <div class=class id="cameras-sync-result">
                        {move_tr!("cameras-sync-done", {"added" => added, "replaced" => replaced, "removed" => removed})}
                        " "{failures}
                    </div>
                }
                .into_any()
            }
        })
    };

    view! {
        <Transition fallback=|| ()>
            {move || {
                matches!(list.get(), Some(Err(()))).then(|| {
                    view! { <div class="vc-alert vc-alert--error" role="alert" id="crud-load-error">{move_tr!("crud-load-error")}</div> }
                })
            }}
            <p class="vc-btn-row">
                <leptos_router::components::A href="/surveillance/discovery" attr:class="vc-btn vc-btn--ghost" attr:id="cameras-discovery-link">{move_tr!("discovery-title")}</leptos_router::components::A>
                <button type="button" class="vc-btn" id="cameras-sync" on:click=move |_| { sync.dispatch(()); }>
                    {move_tr!("cameras-sync")}
                </button>
            </p>
            {sync_message}
            <CrudTable
                labels=Labels {
                    title: tk!("cameras-title"),
                    add: tk!("cameras-add"),
                    edit: tk!("cameras-edit"),
                    empty: tk!("cameras-empty"),
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
