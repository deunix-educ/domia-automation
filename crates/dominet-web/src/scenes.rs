// SPDX-License-Identifier: AGPL-3.0-or-later
//! Scenes (R4 lot 2, ADR 0031) : fonctions serveur, page `/automation/scenes` et bouton d'activation (partage avec le widget
//! scene du tableau de bord).
//!
//! Tout compte connecte active une scene (comme en Reference : la famille lance « Fermeture du soir ») ; seuls les
//! administrateurs les modifient (`scenes_admin`).

use dominet_proto::{CommandOutcome, SceneDto, SceneOutcome};
use leptos::prelude::*;
use leptos_fluent::{I18n, move_tr, tr};
use leptos_router::components::A;

use crate::guard::RequireAuth;
use crate::layout::use_hydrated;

#[cfg(feature = "ssr")]
pub(crate) async fn scene_dtos(
    state: &dominet_server::AppState,
    only_active: bool,
) -> Result<Vec<SceneDto>, ServerFnError> {
    use std::collections::HashMap;

    use dominet_domain::repo::{DeviceFilter, DeviceRepo, LocationRepo, SceneRepo};
    use dominet_proto::SceneActionDto;

    use crate::server_fns::internal;

    let scenes = state
        .store
        .scenes()
        .list()
        .await
        .map_err(|e| internal("scene list", &e))?;
    let devices: HashMap<i64, String> = state
        .store
        .devices()
        .list(&DeviceFilter::default())
        .await
        .map_err(|e| internal("device list", &e))?
        .into_iter()
        .map(|d| (d.id.0, d.friendly_name))
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
    Ok(scenes
        .into_iter()
        .filter(|s| s.active || !only_active)
        .map(|s| SceneDto {
            id: s.id.0,
            name: s.name,
            icon: s.icon,
            description: s.description,
            location_id: s.location.map(|l| l.0.to_string()).unwrap_or_default(),
            location: s
                .location
                .and_then(|l| locations.get(&l.0).cloned())
                .unwrap_or_default(),
            active: s.active,
            actions: s
                .actions
                .into_iter()
                .map(|a| SceneActionDto {
                    device_id: a.device.0,
                    device: devices.get(&a.device.0).cloned().unwrap_or_default(),
                    payload: a.payload.to_string(),
                })
                .collect(),
        })
        .collect())
}

/// Scenes actives, par nom (tout compte connecte).
///
/// # Errors
/// Refus (401) sans session ; erreur interne si la base echoue.
#[server(name = ListScenes, prefix = "/sf", endpoint = "scenes/list")]
pub async fn list_scenes() -> Result<Vec<SceneDto>, ServerFnError> {
    use crate::server_fns::require_user;

    let state = expect_context::<dominet_server::AppState>();
    require_user(&state).await?;
    scene_dtos(&state, true).await
}

/// Active une scene : ses commandes sont publiees dans l'ordre ; une commande en echec n'arrete pas les suivantes.
///
/// # Errors
/// Refus (401) sans session.
#[server(name = ActivateScene, prefix = "/sf", endpoint = "scenes/activate")]
pub async fn activate_scene(id: i64) -> Result<SceneOutcome, ServerFnError> {
    use dominet_domain::scene::SceneId;

    use crate::server_fns::require_user;

    let state = expect_context::<dominet_server::AppState>();
    let user = require_user(&state).await?;
    let outcome = dominet_server::scenes::activate_by_id(&state, SceneId(id)).await;
    tracing::info!(
        user_id = user.id.0,
        scene_id = id,
        "scene activation requested"
    );
    Ok(outcome)
}

/// Texte de l'issue d'une activation (vide si tout est parti).
fn failure_text(outcome: CommandOutcome) -> String {
    match outcome {
        CommandOutcome::Sent => String::new(),
        CommandOutcome::NotFound => tr!("scene-failure-notfound"),
        CommandOutcome::Invalid => tr!("scene-failure-invalid"),
        CommandOutcome::Unavailable => tr!("scene-failure-unavailable"),
    }
}

/// Bouton d'activation et retour (✓ / ✗ avec les commandes en echec). `uid` rend les identifiants uniques sur la page.
#[component]
pub fn SceneButton(
    id: i64,
    #[prop(into)] uid: String,
    /// Texte du bouton.
    #[prop(into)]
    label: Signal<String>,
) -> impl IntoView {
    let ready = use_hydrated();
    let action = Action::new(move |scene: &i64| {
        let scene = *scene;
        async move { activate_scene(scene).await }
    });
    let report = move || match action.value().get() {
        None => None,
        Some(Ok(SceneOutcome::Activated { failed, .. })) if failed.is_empty() => {
            Some((true, tr!("scene-done")))
        }
        Some(Ok(SceneOutcome::Activated { failed, .. })) => Some((
            false,
            failed
                .iter()
                .map(|f| format!("{} : {}", f.device, failure_text(f.outcome)))
                .collect::<Vec<_>>()
                .join(" ; "),
        )),
        Some(Ok(SceneOutcome::NotFound)) => Some((false, tr!("scene-gone"))),
        Some(Err(_)) => Some((false, tr!("scene-failed"))),
    };
    view! {
        <div class="vc-scene-run">
            <button
                type="button"
                class="vc-btn vc-btn--primary scene-activate"
                id=format!("scene-activate-{uid}")
                disabled=move || !ready.get() || action.pending().get()
                on:click=move |_| {
                    action.dispatch(id);
                }
            >
                {move || label.get()}
            </button>
            {move || {
                report()
                    .map(|(ok, text)| {
                        view! {
                            <span
                                class="vc-scene-result"
                                class:vc-scene-result--ok=ok
                                class:vc-scene-result--ko=!ok
                                id=format!("scene-result-{uid}")
                                role="status"
                            >
                                {if ok { format!("✓ {text}") } else { format!("✗ {text}") }}
                            </span>
                        }
                    })
            }}
        </div>
    }
}

/// Corps d'un widget scene du tableau de bord.
pub(crate) fn scene_body(w: &dominet_proto::WidgetDto) -> AnyView {
    let uid = format!("w{}", w.id);
    match &w.scene {
        Some(s) if s.active => view! {
            <div class="vc-widget-body">
                <div class="vc-widget-scene" id=format!("scene-body-{}", w.id)>
                    <div class="vc-scene-name">{format!("{} {}", s.icon, s.name).trim().to_string()}</div>
                    {(!s.description.is_empty()).then(|| view! { <div class="vc-muted vc-scene-desc">{s.description.clone()}</div> })}
                    <SceneButton id=s.id uid label=Signal::derive(|| tr!("scene-activate"))/>
                </div>
            </div>
        }
        .into_any(),
        Some(_) => placeholder_text(w.id, tr!("scene-inactive")),
        None => placeholder_text(w.id, tr!("scene-gone")),
    }
}

fn placeholder_text(widget: i64, text: String) -> AnyView {
    view! {
        <div class="vc-widget-body">
            <div class="vc-widget-placeholder vc-muted" id=format!("scene-problem-{widget}")>{text}</div>
        </div>
    }
    .into_any()
}

#[component]
pub fn ScenesPage() -> impl IntoView {
    view! {
        <RequireAuth>
            <main class="vc-container">
                <ScenesContent/>
            </main>
        </RequireAuth>
    }
}

#[component]
fn ScenesContent() -> impl IntoView {
    let i18n = expect_context::<I18n>();
    let list = Resource::new(|| (), |()| async { list_scenes().await.map_err(|_| ()) });
    view! {
        <div class="vc-page-header">
            <h2>{move_tr!("scenes-title")}</h2>
            <div class="vc-header-actions">
                <A href="/automation/" attr:class="vc-btn vc-btn--ghost" attr:id="scenes-to-devices">{move_tr!("scenes-to-devices")}</A>
            </div>
        </div>
        <Transition fallback=|| ()>
            {move || match list.get() {
                None => ().into_any(),
                Some(Err(())) => view! { <div class="vc-alert vc-alert--error" role="alert" id="scenes-load-error">{move_tr!("crud-load-error")}</div> }.into_any(),
                Some(Ok(scenes)) if scenes.is_empty() => view! { <p class="vc-muted" id="scenes-empty">{move_tr!("scenes-empty")}</p> }.into_any(),
                Some(Ok(scenes)) => view! {
                    <div class="vc-scene-grid" id="scenes-grid">
                        {scenes
                            .into_iter()
                            .map(|s| {
                                let heading = format!("{} {}", s.icon, s.name).trim().to_string();
                                view! {
                                    <section class="vc-card vc-scene-card" data-scene-id=s.id id=format!("scene-card-{}", s.id)>
                                        <h3 class="vc-card-title">{heading}</h3>
                                        {(!s.description.is_empty()).then(|| view! { <p class="vc-muted">{s.description.clone()}</p> })}
                                        {(!s.location.is_empty()).then(|| view! { <p class="vc-muted">{s.location.clone()}</p> })}
                                        <ol class="vc-scene-actions">
                                            {s.actions
                                                .iter()
                                                .map(|a| {
                                                    view! { <li><span class="vc-scene-device">{a.device.clone()}</span>" "<code>{a.payload.clone()}</code></li> }
                                                })
                                                .collect_view()}
                                        </ol>
                                        <SceneButton id=s.id uid=s.id.to_string() label=Signal::derive(move || i18n.tr(tk!("scene-activate")))/>
                                    </section>
                                }
                            })
                            .collect_view()}
                    </div>
                }
                .into_any(),
            }}
        </Transition>
    }
}
