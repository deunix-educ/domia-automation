// SPDX-License-Identifier: AGPL-3.0-or-later
//! Pages des appareils (R3 lot 5, ADR 0029) : la liste filtree `/automation/` (cartes d'etat en direct, commandes) et la page
//! d'un appareil `/automation/devices/<id>` (identite, topics, historique des cles suivies sur 24 h).

use dominet_proto::{
    DeviceCardDto, DeviceDetailDto, DevicesPageDto, GrafanaOutcome, Topic, WidgetDto,
};
use leptos::prelude::*;
use leptos_fluent::{I18n, move_tr, tr};
use leptos_router::components::A;
use leptos_router::hooks::{use_params_map, use_query_map};

use crate::device_fns::{generate_device_dashboard, permit_join_state, set_permit_join};
use crate::device_widget::device_body;
use crate::guard::RequireAuth;
use crate::realtime::Realtime;

// ------------------------------------------------------------------ fonctions serveur

#[cfg(feature = "ssr")]
async fn cards(
    state: &dominet_server::AppState,
    devices: Vec<dominet_domain::device::Device>,
) -> Result<Vec<DeviceCardDto>, ServerFnError> {
    use std::collections::HashMap;

    use dominet_domain::repo::{DeviceGroupRepo, DeviceImageRepo, DeviceLabelRepo, LocationRepo};
    use dominet_proto::{DeviceMetaDto, DeviceStateDto, LabelDto};

    use crate::server_fns::internal;

    let groups: HashMap<i64, String> = state
        .store
        .device_groups()
        .list()
        .await
        .map_err(|e| internal("group list", &e))?
        .into_iter()
        .map(|g| (g.id.0, g.name))
        .collect();
    let locations: HashMap<i64, String> = state
        .store
        .locations()
        .list()
        .await
        .map_err(|e| internal("place list", &e))?
        .into_iter()
        .map(|l| (l.id.0, l.name))
        .collect();
    let labels = state
        .store
        .device_labels()
        .list()
        .await
        .map_err(|e| internal("label list", &e))?;
    let images: HashMap<i64, i64> = state
        .store
        .device_images()
        .with_images()
        .await
        .map_err(|e| internal("image list", &e))?
        .into_iter()
        .map(|(id, at)| (id.0, at.timestamp()))
        .collect();
    let commandable = state.mqtt.is_some();
    Ok(devices
        .into_iter()
        .map(|d| {
            // Seuls les libelles des cles presentes dans l'etat voyagent avec la carte.
            let labels = labels
                .iter()
                .filter(|l| d.state.get(&l.key).is_some())
                .map(|l| LabelDto {
                    key: l.key.clone(),
                    name: l.name.clone(),
                    unit: l.unit.clone(),
                })
                .collect();
            DeviceCardDto {
                id: d.id.0,
                name: d.friendly_name.clone(),
                description: d.description.clone(),
                device_type: d.device_type.as_str().to_string(),
                group: d
                    .group
                    .and_then(|g| groups.get(&g.0).cloned())
                    .unwrap_or_default(),
                location: d
                    .location
                    .and_then(|l| locations.get(&l.0).cloned())
                    .unwrap_or_default(),
                last_seen: d
                    .last_seen
                    .map(dominet_server::timefmt::text)
                    .unwrap_or_default(),
                image_version: images.get(&d.id.0).copied(),
                state: DeviceStateDto {
                    device_id: d.id.0,
                    friendly_name: d.friendly_name,
                    state: d.state,
                    available: d.available,
                },
                meta: DeviceMetaDto {
                    device_type: d.device_type.as_str().to_string(),
                    labels,
                    hidden_keys: d.hidden_keys,
                    commandable,
                },
            }
        })
        .collect())
}

/// Les appareils actifs, filtres par groupe, emplacement et type ; de quoi remplir les filtres.
///
/// # Errors
/// Refus (401) sans session ; erreur interne si la base echoue.
#[server(name = DevicesPage, prefix = "/sf", endpoint = "devices/page")]
pub async fn devices_page(
    group: Option<i64>,
    location: Option<i64>,
    kind: String,
) -> Result<DevicesPageDto, ServerFnError> {
    use std::str::FromStr;

    use dominet_domain::device::{DeviceGroupId, DeviceType};
    use dominet_domain::location::LocationId;
    use dominet_domain::repo::{DeviceFilter, DeviceGroupRepo, DeviceRepo, LocationRepo};

    use crate::server_fns::{internal, require_user};

    let state = expect_context::<dominet_server::AppState>();
    require_user(&state).await?;
    let filter = DeviceFilter {
        group: group.map(DeviceGroupId),
        location: location.map(LocationId),
        device_type: DeviceType::from_str(&kind).ok(),
        active_only: true,
    };
    let devices = state
        .store
        .devices()
        .list(&filter)
        .await
        .map_err(|e| internal("device list", &e))?;
    let groups = state
        .store
        .device_groups()
        .list()
        .await
        .map_err(|e| internal("group list", &e))?
        .into_iter()
        .map(|g| (g.id.0, g.name))
        .collect();
    let locations = state
        .store
        .locations()
        .list()
        .await
        .map_err(|e| internal("place list", &e))?
        .into_iter()
        .filter(|l| l.active)
        .map(|l| (l.id.0, l.name))
        .collect();
    Ok(DevicesPageDto {
        devices: cards(&state, devices).await?,
        groups,
        locations,
        types: DeviceType::ALL
            .iter()
            .map(|t| t.as_str().to_string())
            .collect(),
        permit_join: state.permit_join.load(std::sync::atomic::Ordering::Relaxed),
        commandable: state.mqtt.is_some(),
    })
}

/// Un appareil, son historique des 24 dernieres heures (cles suivies numeriques) ; `None` s'il n'existe pas.
///
/// # Errors
/// Refus (401) sans session ; erreur interne si la base echoue.
#[server(name = DeviceDetail, prefix = "/sf", endpoint = "devices/detail")]
pub async fn device_detail(id: i64) -> Result<Option<DeviceDetailDto>, ServerFnError> {
    use dominet_domain::device::DeviceId;
    use dominet_domain::repo::{DeviceLabelRepo, DeviceRepo, StateLogRepo};
    use dominet_proto::SeriesDto;

    use crate::server_fns::{internal, require_user};

    let state = expect_context::<dominet_server::AppState>();
    require_user(&state).await?;
    let Some(device) = state
        .store
        .devices()
        .get(DeviceId(id))
        .await
        .map_err(|e| internal("reading a device", &e))?
    else {
        return Ok(None);
    };
    let labels = state
        .store
        .device_labels()
        .list()
        .await
        .map_err(|e| internal("label list", &e))?;
    let since = chrono::Utc::now() - chrono::Duration::hours(24);
    let mut series = vec![];
    for key in &device.tracked_keys {
        let points = state
            .store
            .state_log()
            .points(device.id, key, since, 500)
            .await
            .map_err(|e| internal("reading the history", &e))?;
        let points: Vec<(i64, f64)> = points
            .into_iter()
            .filter_map(|p| p.value_num.map(|v| (p.ts.timestamp(), v)))
            .collect();
        if points.is_empty() {
            continue;
        }
        let label = labels.iter().find(|l| &l.key == key);
        series.push(SeriesDto {
            key: key.clone(),
            name: label.map_or_else(|| key.clone(), |l| l.name.clone()),
            unit: label.map(|l| l.unit.clone()).unwrap_or_default(),
            points,
        });
    }
    let detail = DeviceDetailDto {
        ieee: device.ieee.clone(),
        vendor: device.vendor.clone(),
        model_id: device.model_id.clone(),
        state_topic: device.state_topic(),
        command_topic: device.command_topic(),
        tracked_keys: device.tracked_keys.clone(),
        active: device.active,
        grafana_uid: if device.grafana_dashboard {
            device.grafana_uid()
        } else {
            String::new()
        },
        card: cards(&state, vec![device])
            .await?
            .pop()
            .ok_or_else(|| ServerFnError::new("device not found"))?,
        series,
    };
    Ok(Some(detail))
}

// ------------------------------------------------------------------ affichage

/// Points d'une polyligne (`x,y x,y ...`) dans un cadre `largeur` x `hauteur`, l'axe du temps allant du premier au dernier
/// point et celui des valeurs du minimum au maximum (une serie constante est tracee au milieu).
#[must_use]
pub fn sparkline_points(points: &[(i64, f64)], width: f64, height: f64) -> String {
    let (Some(first), Some(last)) = (points.first(), points.last()) else {
        return String::new();
    };
    // Secondes depuis le premier point (une serie de 24 h tient largement dans un i32).
    let secs = |t: i64| f64::from(i32::try_from(t - first.0).unwrap_or(i32::MAX));
    let span = secs(last.0);
    let (lo, hi) = points
        .iter()
        .fold((f64::MAX, f64::MIN), |(lo, hi), (_, v)| {
            (lo.min(*v), hi.max(*v))
        });
    let margin = 2.0;
    points
        .iter()
        .map(|(t, v)| {
            let x = if span > 0.0 {
                secs(*t) / span * width
            } else {
                width / 2.0
            };
            let y = if hi > lo {
                height - margin - (v - lo) / (hi - lo) * (height - 2.0 * margin)
            } else {
                height / 2.0
            };
            format!("{x:.1},{y:.1}")
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn card_as_widget(c: &DeviceCardDto) -> WidgetDto {
    WidgetDto {
        id: c.id,
        widget_type: "device".into(),
        title: c.name.clone(),
        presentation: String::new(),
        device_id: Some(c.id),
        col: 0,
        row: 0,
        width: 3,
        height: 3,
        content_html: String::new(),
        iframe_src: None,
        iframe_problem: String::new(),
        camera: None,
        device: Some(c.state.clone()),
        device_meta: Some(c.meta.clone()),
        scene: None,
    }
}

/// Etat en direct de l'appareil (abonnement au sujet `device:<id>`, dernier etat de la page au depart).
fn live_state(c: &DeviceCardDto) -> Signal<Option<dominet_proto::DeviceStateDto>> {
    let rt = expect_context::<Realtime>();
    let id = c.id;
    Effect::new(move |_| rt.subscribe(Topic::Device(id)));
    on_cleanup(move || rt.unsubscribe(&Topic::Device(id)));
    // Un appareil qui n'a jamais rien publie (etat vide) attend son premier message.
    let initial =
        (c.state.state.as_object().is_some_and(|o| !o.is_empty())).then(|| c.state.clone());
    Signal::derive(move || rt.device(id).or_else(|| initial.clone()))
}

fn image_src(c: &DeviceCardDto) -> Option<String> {
    c.image_version
        .map(|v| format!("/media/devices/{}?v={v}", c.id))
}

#[component]
fn DeviceCard(card: DeviceCardDto) -> impl IntoView {
    let state = live_state(&card);
    let widget = card_as_widget(&card);
    let id = card.id;
    let name = card.name.clone();
    let meta_line: Vec<String> = [card.group.clone(), card.location.clone()]
        .into_iter()
        .filter(|s| !s.is_empty())
        .collect();
    view! {
        <article
            class="vc-widget vc-device-card"
            class:vc-widget--on=move || state.get().is_some_and(|s| s.available && s.state.get("state").and_then(|v| v.as_str()) == Some("ON"))
            class:vc-widget--offline=move || state.get().is_some_and(|s| !s.available)
            id=format!("device-card-{id}")
            data-device-id=id
        >
            <div class="vc-widget-header">
                <A href=format!("/automation/devices/{id}")>
                    <span class="vc-widget-header-title">{name}</span>
                </A>
                <span class="vc-dev-id vc-muted" title=move || tr!("devices-id")>{format!("#{id}")}</span>
                <span
                    class=move || if state.get().is_some_and(|s| s.available) { "vc-dev-dot vc-dev-dot--on" } else { "vc-dev-dot vc-dev-dot--off" }
                ></span>
            </div>
            {image_src(&card).map(|src| view! { <img class="vc-device-img" src=src alt=""/> })}
            {device_body(&widget, state)}
            {(!meta_line.is_empty()).then(|| view! { <div class="vc-dev-meta vc-muted">{meta_line.join(" · ")}</div> })}
        </article>
    }
}

fn parse_id(raw: Option<String>) -> Option<i64> {
    raw.and_then(|v| v.parse::<i64>().ok()).filter(|n| *n > 0)
}

/// Page `/automation/` : les appareils actifs, avec filtres (groupe, emplacement, type) en GET.
#[component]
pub fn DevicesPage() -> impl IntoView {
    view! {
        <RequireAuth>
            <main class="vc-container">
                <DevicesList/>
            </main>
        </RequireAuth>
    }
}

#[component]
fn DevicesList() -> impl IntoView {
    let query = use_query_map();
    let filters = move || {
        query.with(|q| {
            (
                parse_id(q.get("group")),
                parse_id(q.get("location")),
                q.get("type").unwrap_or_default(),
            )
        })
    };
    let page = Resource::new(filters, |(group, location, kind)| async move {
        devices_page(group, location, kind).await.map_err(|_| ())
    });
    view! {
        <div class="vc-page-header">
            <h2>{move_tr!("devices-page-title")}</h2>
        </div>
        <Transition fallback=|| ()>
            {move || match page.get() {
                None => ().into_any(),
                Some(Err(())) => {
                    view! { <div class="vc-alert vc-alert--error" role="alert" id="devices-load-error">{move_tr!("crud-load-error")}</div> }
                        .into_any()
                }
                Some(Ok(p)) => view! { <DevicesContent page=p/> }.into_any(),
            }}
        </Transition>
    }
}

#[component]
fn DevicesContent(page: DevicesPageDto) -> impl IntoView {
    let query = use_query_map();
    let current = move |key: &str| query.with(|q| q.get(key).unwrap_or_default());
    let admin = crate::dashboard::is_admin();
    let commandable = page.commandable;
    let join = RwSignal::new(page.permit_join);
    let toggle = Action::new(move |enabled: &bool| {
        let enabled = *enabled;
        async move {
            if let Ok(dominet_proto::CommandOutcome::Sent) = set_permit_join(enabled).await {
                join.set(enabled);
            }
            permit_join_state().await.unwrap_or(enabled)
        }
    });
    let options = |items: Vec<(i64, String)>, key: &'static str| {
        items
            .into_iter()
            .map(|(id, name)| {
                let value = id.to_string();
                let v = value.clone();
                view! { <option value=value selected=move || current(key) == v>{name}</option> }
            })
            .collect_view()
    };
    let type_options = page
        .types
        .iter()
        .map(|t| {
            let (value, v) = (t.clone(), t.clone());
            let label = type_label(t);
            view! { <option value=value selected=move || current("type") == v>{move || label()}</option> }
        })
        .collect_view();
    let empty = page.devices.is_empty();
    view! {
        <form method="get" action="/automation/" class="vc-filters" id="devices-filters">
            <select name="group" id="filter-group" aria-label=move || tr!("devices-group")>
                <option value="">{move_tr!("devices-all-groups")}</option>
                {options(page.groups.clone(), "group")}
            </select>
            <select name="location" id="filter-location" aria-label=move || tr!("devices-location")>
                <option value="">{move_tr!("devices-all-locations")}</option>
                {options(page.locations.clone(), "location")}
            </select>
            <select name="type" id="filter-type" aria-label=move || tr!("devices-type")>
                <option value="">{move_tr!("devices-all-types")}</option>
                {type_options}
            </select>
            <button type="submit" class="vc-btn vc-btn--ghost" id="filter-apply">{move_tr!("devices-filter")}</button>
            <A href="/automation/scenes" attr:class="vc-btn vc-btn--ghost" attr:id="devices-to-scenes">{move_tr!("devices-to-scenes")}</A>
            <A href="/automation/energy" attr:class="vc-btn vc-btn--ghost" attr:id="devices-to-energy">{move_tr!("devices-to-energy")}</A>
        </form>
        <Show when=move || admin.get()>
            <div class="vc-card vc-permit" id="permit-join">
                <span id="permit-state">
                    {move || if join.get() { tr!("devices-permit-open") } else { tr!("devices-permit-closed") }}
                </span>
                <button
                    type="button"
                    class="vc-btn vc-btn--ghost"
                    id="permit-toggle"
                    disabled=move || !commandable || toggle.pending().get()
                    on:click=move |_| { toggle.dispatch(!join.get_untracked()); }
                >
                    {move || if join.get() { tr!("devices-permit-close") } else { tr!("devices-permit-open-action") }}
                </button>
            </div>
        </Show>
        {empty.then(|| view! { <p class="vc-muted" id="devices-empty">{move_tr!("devices-none")}</p> })}
        <div class="vc-device-grid" id="devices-grid">
            {page.devices.into_iter().map(|c| view! { <DeviceCard card=c/> }).collect_view()}
        </div>
    }
}

/// Libelle traduit d'un type d'appareil.
fn type_label(kind: &str) -> impl Fn() -> String + 'static {
    let i18n = expect_context::<I18n>();
    let key = match kind {
        "sensor" => "devtype-sensor",
        "binary_sensor" => "devtype-binary-sensor",
        "switch" => "devtype-switch",
        "light" => "devtype-light",
        "cover" => "devtype-cover",
        "climate" => "devtype-climate",
        "remote" => "devtype-remote",
        _ => "devtype-generic",
    };
    move || i18n.tr(key)
}

/// Page `/automation/devices/<id>`.
#[component]
pub fn DevicePage() -> impl IntoView {
    view! {
        <RequireAuth>
            <main class="vc-container">
                <DeviceDetail/>
            </main>
        </RequireAuth>
    }
}

#[component]
fn DeviceDetail() -> impl IntoView {
    let params = use_params_map();
    let id = move || params.with(|p| parse_id(p.get("id")));
    let detail = Resource::new(id, |id| async move {
        match id {
            Some(id) => device_detail(id).await.map_err(|_| ()),
            None => Ok(None),
        }
    });
    view! {
        <Transition fallback=|| ()>
            {move || match detail.get() {
                None => ().into_any(),
                Some(Err(())) => {
                    view! { <div class="vc-alert vc-alert--error" role="alert" id="devices-load-error">{move_tr!("crud-load-error")}</div> }
                        .into_any()
                }
                Some(Ok(None)) => {
                    #[cfg(feature = "ssr")]
                    expect_context::<leptos_axum::ResponseOptions>()
                        .set_status(axum::http::StatusCode::NOT_FOUND);
                    view! { <p class="vc-muted" id="device-missing">{move_tr!("devices-missing")}</p> }.into_any()
                }
                Some(Ok(Some(d))) => view! { <DeviceDetailView detail=d/> }.into_any(),
            }}
        </Transition>
    }
}

#[component]
fn DeviceDetailView(detail: DeviceDetailDto) -> impl IntoView {
    let card = detail.card.clone();
    let state = live_state(&card);
    let widget = card_as_widget(&card);
    let admin = crate::dashboard::is_admin();
    let i18n = expect_context::<I18n>();
    let row = move |label: &'static str, value: String| {
        (!value.is_empty()).then(|| {
            view! {
                <dt>{move || i18n.tr(label)}</dt>
                <dd>{value}</dd>
            }
        })
    };
    let series = detail
        .series
        .iter()
        .map(|s| {
            let polyline = sparkline_points(&s.points, 240.0, 48.0);
            let last = s.points.last().map_or(0.0, |p| p.1);
            let (lo, hi) = s.points.iter().fold((f64::MAX, f64::MIN), |(lo, hi), p| (lo.min(p.1), hi.max(p.1)));
            let unit = s.unit.clone();
            view! {
                <div class="vc-series" data-key=s.key.clone()>
                    <div class="vc-series-head">
                        <span>{s.name.clone()}</span>
                        <span class="vc-muted">{format!("{last:.1} {unit} (min {lo:.1}, max {hi:.1})")}</span>
                    </div>
                    <svg viewBox="0 0 240 48" class="vc-sparkline" role="img" aria-label=s.name.clone()>
                        <polyline points=polyline fill="none" stroke="currentColor" stroke-width="1.5"/>
                    </svg>
                </div>
            }
        })
        .collect_view();
    let has_series = !detail.series.is_empty();
    let id = card.id;
    let has_image = card.image_version.is_some();
    let has_tracked = !detail.tracked_keys.is_empty();
    let query = use_query_map();
    let image_outcome = move || query.with(|q| q.get("image")).unwrap_or_default();
    let dashboard = Action::new(move |(): &()| async move { generate_device_dashboard(id).await });
    let dashboard_message = move || match dashboard.value().get() {
        None => String::new(),
        Some(Ok(GrafanaOutcome::Written)) => tr!("devices-grafana-written"),
        Some(Ok(GrafanaOutcome::NoTrackedKeys)) => tr!("devices-grafana-no-keys"),
        Some(Ok(GrafanaOutcome::NotConfigured)) => tr!("devices-grafana-not-configured"),
        Some(Ok(GrafanaOutcome::NotFound)) => tr!("crud-error-not-found"),
        Some(Err(_)) => tr!("crud-error-failed"),
    };
    let tracked = detail.tracked_keys.join(", ");
    let grafana_uid = detail.grafana_uid.clone();
    view! {
        <div class="vc-page-header">
            <h2 id="device-title">{card.name.clone()}</h2>
            <div class="vc-header-actions">
                <A href="/automation/" attr:class="vc-btn vc-btn--ghost">{move_tr!("devices-back")}</A>
                <Show when=move || admin.get()>
                    <A href="/admin/devices" attr:class="vc-btn vc-btn--ghost">{move_tr!("crud-edit")}</A>
                </Show>
            </div>
        </div>
        <section class="vc-card" id="device-live">
            {image_src(&card).map(|src| view! { <img class="vc-device-img" src=src alt=""/> })}
            {device_body(&widget, state)}
        </section>
        <section class="vc-card" id="device-info">
            <dl class="vc-dl">
                {row("devices-id", id.to_string())}
                {row("devices-type", card.device_type.clone())}
                {row("devices-group", card.group.clone())}
                {row("devices-location", card.location.clone())}
                {row("devices-description", card.description.clone())}
                {row(tk!("devices-ieee"), detail.ieee.clone())}
                {row(tk!("devices-vendor"), detail.vendor.clone())}
                {row(tk!("devices-model"), detail.model_id.clone())}
                {row(tk!("devices-state-topic"), detail.state_topic.clone())}
                {row(tk!("devices-command-topic"), detail.command_topic.clone())}
                {row("devices-tracked", tracked)}
                {row(tk!("devices-last-seen"), card.last_seen.clone())}
                {row("devices-grafana", grafana_uid)}
            </dl>
        </section>
        <Show when=move || admin.get()>
            <section class="vc-card" id="device-admin">
                <h3 class="vc-card-title">{move_tr!("devices-admin-tools")}</h3>
                {move || {
                    let key = match image_outcome().as_str() {
                        "saved" => Some(tk!("devices-image-saved")),
                        "deleted" => Some(tk!("devices-image-deleted")),
                        "unsupported" => Some(tk!("devices-image-unsupported")),
                        "toolarge" => Some(tk!("devices-image-too-large")),
                        "missing" => Some(tk!("devices-image-missing")),
                        _ => None,
                    };
                    key.map(|k| {
                        let i18n = expect_context::<I18n>();
                        view! { <div class="vc-alert" role="status" id="image-result">{move || i18n.tr(k)}</div> }
                    })
                }}
                <form method="post" action=format!("/api/devices/{id}/image") enctype="multipart/form-data" id="image-form" class="vc-inline-form">
                    <label for="image-file">{move_tr!("devices-image")}</label>
                    <input type="file" id="image-file" name="image" accept="image/jpeg,image/png,image/webp,image/gif" required/>
                    <button type="submit" class="vc-btn vc-btn--primary">{move_tr!("devices-image-upload")}</button>
                </form>
                {has_image.then(|| view! {
                    <form method="post" action=format!("/api/devices/{id}/image/delete") id="image-delete-form" class="vc-inline-form">
                        <button type="submit" class="vc-btn vc-btn--danger">{move_tr!("devices-image-delete")}</button>
                    </form>
                })}
                <div class="vc-inline-form">
                    <button
                        type="button"
                        class="vc-btn vc-btn--ghost"
                        id="grafana-generate"
                        disabled=move || !has_tracked || dashboard.pending().get()
                        on:click=move |_| { dashboard.dispatch(()); }
                    >
                        {move_tr!("devices-grafana-generate")}
                    </button>
                    <span class="vc-muted" id="grafana-result" role="status">{dashboard_message}</span>
                </div>
            </section>
        </Show>
        <section class="vc-card" id="device-history">
            <h3 class="vc-card-title">{move_tr!("devices-history")}</h3>
            {if has_series { series.into_any() } else { view! { <p class="vc-muted" id="device-no-history">{move_tr!("devices-no-history")}</p> }.into_any() }}
        </section>
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_sparkline_spans_the_frame_with_the_minimum_at_the_bottom() {
        let p = sparkline_points(&[(0, 10.0), (50, 20.0), (100, 15.0)], 200.0, 40.0);
        assert_eq!(p, "0.0,38.0 100.0,2.0 200.0,20.0");
    }

    #[test]
    fn flat_and_single_point_series_are_centered() {
        assert_eq!(
            sparkline_points(&[(0, 5.0), (10, 5.0)], 100.0, 40.0),
            "0.0,20.0 100.0,20.0"
        );
        assert_eq!(sparkline_points(&[(7, 5.0)], 100.0, 40.0), "50.0,20.0");
        assert_eq!(sparkline_points(&[], 100.0, 40.0), "");
    }
}
