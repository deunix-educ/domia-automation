// SPDX-License-Identifier: AGPL-3.0-or-later
//! Page de consultation des cameras : `/surveillance` (R6 lot 2). Tout compte connecte la voit ; le live, les snapshots et les
//! enregistrements s'y ajoutent aux lots suivants.

use leptos::prelude::*;
use leptos_fluent::move_tr;
use leptos_router::components::A;

use crate::camera_fns::list_camera_cards;
use crate::guard::RequireAuth;

#[component]
pub fn SurveillancePage() -> impl IntoView {
    view! {
        <RequireAuth>
            <main class="vc-container">
                <Cameras/>
            </main>
        </RequireAuth>
    }
}

#[component]
fn Cameras() -> impl IntoView {
    let list = Resource::new(
        || (),
        |()| async { list_camera_cards().await.map_err(|_| ()) },
    );
    // Les vignettes se rafraichissent toutes les 5 s (navigateur seulement).
    let tick = RwSignal::new(0u32);
    #[cfg(feature = "hydrate")]
    {
        if let Ok(handle) = set_interval_with_handle(
            move || tick.update(|t| *t = t.wrapping_add(1)),
            std::time::Duration::from_secs(5),
        ) {
            on_cleanup(move || handle.clear());
        }
    }
    view! {
        <h1>{move_tr!("surveillance-title")}</h1>
        <p class="vc-btn-row">
            <A href="/surveillance/snapshots" attr:class="vc-btn vc-btn--ghost">{move_tr!("snapshots-title")}</A>
            <A href="/surveillance/events" attr:class="vc-btn vc-btn--ghost" attr:id="surveillance-events-link">{move_tr!("events-title")}</A>
        </p>
        <Transition fallback=|| ()>
            {move || match list.get() {
                Some(Err(())) => view! { <div class="vc-alert vc-alert--error" role="alert" id="surveillance-error">{move_tr!("crud-load-error")}</div> }.into_any(),
                Some(Ok(cards)) if cards.is_empty() => view! { <p id="surveillance-empty">{move_tr!("surveillance-empty")}</p> }.into_any(),
                Some(Ok(cards)) => view! {
                    <ul class="vc-camera-list" id="surveillance-list">
                        {cards.into_iter().map(|c| { let (href, thumb, name, place) = (format!("/surveillance/live/{}", c.uuid), c.uuid.clone(), c.name.clone(), c.location.clone()); view! {
                            <li class="vc-camera" data-camera=c.uuid.clone()>
                                <A href=href attr:class="vc-camera-link">
                                    <Thumbnail uuid=thumb tick/>
                                    <strong class="vc-camera-name">{name}</strong>
                                    {(!place.is_empty()).then(|| view! { <span class="vc-camera-place">{place}</span> })}
                                </A>
                            </li>
                        } }).collect_view()}
                    </ul>
                }.into_any(),
                None => ().into_any(),
            }}
        </Transition>
    }
}

/// Derniere image d'une camera ; un cadre sombre si elle n'en a pas encore.
#[component]
fn Thumbnail(uuid: String, tick: RwSignal<u32>) -> impl IntoView {
    let missing = RwSignal::new(false);
    view! {
        <span class="vc-camera-thumb" class:vc-camera-thumb--missing=move || missing.get()>
            <img
                alt=""
                loading="lazy"
                src=move || format!("/surveillance/camera/{uuid}/snapshot/?t={}", tick.get())
                on:error=move |_| missing.set(true)
                on:load=move |_| missing.set(false)
            />
        </span>
    }
}
