// SPDX-License-Identifier: AGPL-3.0-or-later
//! Administration des widgets d'un tableau : `/admin/widgets?d=<id>`. Le formulaire n'affiche que les champs utiles au type
//! choisi (appareil pour `device`, scene pour `scene`, camera pour `camera`, contenu pour `text`, configuration JSON pour
//! `grafana` et `camera`).

use dominet_proto::{WidgetAdminDto, WidgetInput};
use leptos::prelude::*;
use leptos_fluent::move_tr;
use leptos_router::NavigateOptions;
use leptos_router::components::A;
use leptos_router::hooks::{use_navigate, use_query_map};

use crate::crud::{CrudResult, CrudTable, FieldKind, FieldSpec, Labels, Row, Values};
use crate::dashboard_fns::{delete_widget, list_dashboards_admin, save_widget, widgets_page};
use crate::guard::RequireAdmin;

const TYPES: &[(&str, &str)] = &[
    ("text", tk!("widget-type-text")),
    ("device", tk!("widget-type-device")),
    ("grafana", tk!("widget-type-grafana")),
    ("camera", tk!("widget-type-camera")),
    ("scene", tk!("widget-type-scene")),
    ("grille", tk!("widget-type-grille")),
];

const PRESENTATIONS: &[(&str, &str)] = &[
    ("", tk!("pres-auto")),
    ("device/sensor", tk!("pres-sensor")),
    ("device/binary_sensor", tk!("pres-binary-sensor")),
    ("device/switch", tk!("pres-switch")),
    ("device/light", tk!("pres-light")),
    ("device/cover", tk!("pres-cover")),
    ("device/climate", tk!("pres-climate")),
    ("device/fan", tk!("pres-fan")),
    ("device/lock", tk!("pres-lock")),
    ("device/generic", tk!("pres-generic")),
    ("device/weather", tk!("pres-weather")),
    ("camera", tk!("widget-type-camera")),
    ("grafana", tk!("widget-type-grafana")),
    ("scene", tk!("widget-type-scene")),
    ("text", tk!("widget-type-text")),
];

fn row(w: &WidgetAdminDto) -> Row {
    Row {
        id: w.id,
        values: Values::from([
            ("title", w.title.clone()),
            ("widget_type", w.widget_type.clone()),
            ("presentation", w.presentation.clone()),
            ("device_id", w.device_id.clone()),
            ("scene_id", w.scene_id.clone()),
            ("camera_uuid", w.camera_uuid.clone()),
            ("config", w.config.clone()),
            ("content", w.content.clone()),
            ("col", w.col.to_string()),
            ("row", w.row.to_string()),
            ("width", w.width.to_string()),
            ("height", w.height.to_string()),
        ]),
    }
}

/// Champs du formulaire utiles au type de widget choisi.
fn visible((name, values): (&'static str, Values)) -> bool {
    let kind = values.get("widget_type").map_or("", String::as_str);
    match name {
        "device_id" => matches!(kind, "device" | "grafana"),
        "scene_id" => kind == "scene",
        "camera_uuid" => kind == "camera",
        "content" => kind == "text",
        "config" => matches!(kind, "grafana" | "camera"),
        "presentation" => kind != "grille",
        _ => true,
    }
}

fn fields() -> Vec<FieldSpec> {
    let base = |name, label, kind| FieldSpec {
        name,
        label,
        kind,
        default: "",
        required: false,
        max_len: None,
        hint: None,
        stored_flag: None,
        create_only: false,
        in_table: false,
    };
    let number = |name, label, default, max: i64, in_table| FieldSpec {
        default,
        in_table,
        ..base(name, label, FieldKind::Number { min: 0, max })
    };
    vec![
        FieldSpec {
            max_len: Some(64),
            in_table: true,
            ..base("title", tk!("widgets-title-field"), FieldKind::Text)
        },
        FieldSpec {
            default: "text",
            required: true,
            in_table: true,
            ..base("widget_type", tk!("widgets-type"), FieldKind::Select(TYPES))
        },
        FieldSpec {
            ..base(
                "presentation",
                tk!("widgets-presentation"),
                FieldKind::Select(PRESENTATIONS),
            )
        },
        // Facultatifs pour le navigateur : seul le serveur sait quand une reference est requise (selon le type).
        // Liste deroulante des appareils existants (R3) : le numero n'a plus a etre connu.
        FieldSpec::new("device_id", tk!("widgets-device"), FieldKind::Choice).column(),
        FieldSpec::new("scene_id", tk!("widgets-scene"), FieldKind::Choice),
        FieldSpec {
            max_len: Some(36),
            hint: Some(tk!("widgets-camera-hint")),
            stored_flag: None,
            ..base("camera_uuid", tk!("widgets-camera"), FieldKind::Text)
        },
        FieldSpec {
            hint: Some(tk!("widgets-config-hint")),
            stored_flag: None,
            ..base("config", tk!("widgets-config"), FieldKind::TextArea)
        },
        FieldSpec {
            max_len: Some(20_000),
            hint: Some(tk!("widgets-content-hint")),
            stored_flag: None,
            ..base("content", tk!("widgets-content"), FieldKind::TextArea)
        },
        number("col", tk!("widgets-col"), "0", 11, true),
        number("row", tk!("widgets-row"), "0", 99, true),
        number("width", tk!("widgets-width"), "2", 12, true),
        number("height", tk!("widgets-height"), "2", 12, true),
    ]
}

#[component]
pub fn WidgetsAdminPage() -> impl IntoView {
    view! {
        <RequireAdmin>
            <main class="vc-container">
                <WidgetsAdmin/>
            </main>
        </RequireAdmin>
    }
}

/// Page : choix du tableau, puis CRUD de ses widgets. Les ressources et le tableau CRUD sont crees **une seule fois** : recreer le
/// composant a chaque rechargement de la liste des tableaux laissait l'ecran sur une instance perimee apres un enregistrement.
#[component]
fn WidgetsAdmin() -> impl IntoView {
    crate::devices_admin::provide_choices(&["device_id", "scene_id"]);
    let query = use_query_map();
    let requested = move || query.with(|q| q.get("d").and_then(|v| v.parse::<i64>().ok()));
    // Liste des tableaux (pour le selecteur seulement).
    let boards = Resource::new(
        || (),
        |()| async { list_dashboards_admin().await.unwrap_or_default() },
    );
    let version = RwSignal::new(0u32);
    // Un seul chargement donne le tableau retenu et ses widgets (aucune dependance entre deux ressources).
    let list = Resource::new(
        move || (requested(), version.get()),
        |(d, _)| async move { widgets_page(d).await.map_err(|_| ()) },
    );
    let current = Memo::new(move |_| match list.get() {
        Some(Ok(p)) => p.current,
        _ => None,
    });
    let rows = Signal::derive(move || match list.get() {
        Some(Ok(p)) => p.widgets.iter().map(row).collect(),
        _ => vec![],
    });
    let save = Action::new(move |(id, v): &(Option<i64>, Values)| {
        let get = |k: &str| v.get(k).cloned().unwrap_or_default();
        let input = WidgetInput {
            id: *id,
            dashboard_id: current.get_untracked().unwrap_or_default(),
            widget_type: get("widget_type"),
            title: get("title"),
            presentation: get("presentation"),
            device_id: get("device_id"),
            scene_id: get("scene_id"),
            camera_uuid: get("camera_uuid"),
            config: get("config"),
            content: get("content"),
            col: get("col"),
            row: get("row"),
            width: get("width"),
            height: get("height"),
        };
        async move {
            save_widget(input)
                .await
                .map_or(CrudResult::Failed, CrudResult::from)
        }
    });
    let remove = Action::new(|id: &i64| {
        let id = *id;
        async move {
            delete_widget(id)
                .await
                .map_or(CrudResult::Failed, CrudResult::from)
        }
    });
    let navigate = use_navigate();
    view! {
        <Transition fallback=|| ()>
            {move || {
                matches!(list.get(), Some(Err(()))).then(|| {
                    view! { <div class="vc-alert vc-alert--error" role="alert" id="crud-load-error">{move_tr!("crud-load-error")}</div> }
                })
            }}
            {move || {
                (matches!(list.get(), Some(Ok(ref p)) if p.current.is_none()))
                    .then(|| {
                        view! {
                            <p class="vc-muted" id="widgets-no-dashboard">{move_tr!("widgets-no-dashboard")}</p>
                            <p><A href="/admin/dashboards">{move_tr!("widgets-create-dashboard")}</A></p>
                        }
                    })
            }}
            <div class="vc-form-group">
                <label for="widgets-dashboard">{move_tr!("widgets-dashboard")}</label>
                <select
                    id="widgets-dashboard"
                    on:change={
                        let navigate = navigate.clone();
                        move |ev| navigate(
                            &format!("/admin/widgets?d={}", event_target_value(&ev)),
                            NavigateOptions::default(),
                        )
                    }
                >
                    {move || {
                        let selected = current.get();
                        boards
                            .get()
                            .unwrap_or_default()
                            .into_iter()
                            .map(|b| {
                                let label = format!("{} — {}", b.owner, b.name);
                                view! { <option value=b.id.to_string() selected=Some(b.id) == selected>{label}</option> }
                            })
                            .collect_view()
                    }}
                </select>
            </div>
            <CrudTable
                labels=Labels {
                    title: tk!("widgets-title"),
                    add: tk!("widgets-add"),
                    edit: tk!("widgets-edit"),
                    empty: tk!("widgets-empty"),
                }
                fields=fields()
                rows
                save
                remove
                field_visible=Callback::new(visible)
                on_change=Callback::new(move |()| version.update(|v| *v += 1))
            />
            <p class="vc-muted" id="widgets-grid-hint">{move_tr!("widgets-grid-hint")}</p>
        </Transition>
    }
}
