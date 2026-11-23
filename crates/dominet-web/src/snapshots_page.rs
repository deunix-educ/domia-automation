// SPDX-License-Identifier: AGPL-3.0-or-later
//! Snapshots gallery: `/surveillance/snapshots` (R6 lot 6, ADR 0048). Any logged-in account views; administrators delete.

use leptos::prelude::*;
use leptos_fluent::move_tr;
use leptos_router::components::A;

use crate::account::format_instant;
use crate::camera_fns::{delete_snapshot, list_camera_cards, list_snapshots};
use crate::guard::RequireAuth;

#[component]
pub fn SnapshotsPage() -> impl IntoView {
    view! {
        <RequireAuth>
            <main class="vc-container">
                <Snapshots/>
            </main>
        </RequireAuth>
    }
}

#[component]
fn Snapshots() -> impl IntoView {
    let admin = crate::dashboard::is_admin();
    let camera = RwSignal::new(String::new());
    let day = RwSignal::new(String::new());
    let version = RwSignal::new(0u32);
    let cameras = Resource::new(
        || (),
        |()| async { list_camera_cards().await.unwrap_or_default() },
    );
    let list = Resource::new(
        move || (camera.get(), day.get(), version.get()),
        |(camera, day, _)| async move { list_snapshots(camera, day).await.map_err(|_| ()) },
    );
    let remove = Action::new(move |id: &i64| {
        let id = *id;
        async move {
            let _ = delete_snapshot(id).await;
            version.update(|v| *v += 1);
        }
    });
    view! {
        <div class="vc-page-header"><h2>{move_tr!("snapshots-title")}</h2></div>
        <p class="vc-btn-row"><A href="/surveillance" attr:class="vc-btn vc-btn--ghost">{move_tr!("live-back")}</A></p>
        <div class="vc-card">
            <div class="vc-snap-filters">
                <div class="vc-form-group">
                    <label for="snap-camera">{move_tr!("snapshots-camera")}</label>
                    <select id="snap-camera" on:change=move |e| camera.set(event_target_value(&e))>
                        <option value="">{move_tr!("snapshots-all-cameras")}</option>
                        <Transition fallback=|| ()>
                            {move || cameras.get().unwrap_or_default().into_iter().map(|c| view! { <option value=c.uuid.clone()>{c.name.clone()}</option> }).collect_view()}
                        </Transition>
                    </select>
                </div>
                <div class="vc-form-group">
                    <label for="snap-day">{move_tr!("snapshots-day")}</label>
                    <input id="snap-day" type="date" on:change=move |e| day.set(event_target_value(&e))/>
                </div>
            </div>
        </div>
        <Transition fallback=|| ()>
            {move || match list.get() {
                None => ().into_any(),
                Some(Err(())) => view! { <div class="vc-alert vc-alert--error" role="alert" id="snapshots-error">{move_tr!("crud-load-error")}</div> }.into_any(),
                Some(Ok(items)) if items.is_empty() => view! { <p id="snapshots-empty">{move_tr!("snapshots-empty")}</p> }.into_any(),
                Some(Ok(items)) => view! {
                    <ul id="snapshots-grid" class="vc-snap-grid">
                        {items.into_iter().map(|s| {
                            let (id, file) = (s.id, format!("/surveillance/snapshots/{}/file/", s.id));
                            view! {
                                <li data-snapshot=id.to_string()>
                                    <a href=file target="_blank" rel="noopener">
                                        <img src=file.clone() alt=s.camera_name.clone() loading="lazy"/>
                                    </a>
                                    <span class="vc-snap-caption"><strong>{s.camera_name.clone()}</strong>" · "{format_instant(&s.taken_at)}</span>
                                    {move || admin.get().then(|| view! {
                                        <button type="button" class="vc-btn vc-btn--danger vc-btn--sm snapshot-delete" on:click=move |_| { remove.dispatch(id); }>
                                            {move_tr!("ptz-preset-remove")}
                                        </button>
                                    })}
                                </li>
                            }
                        }).collect_view()}
                    </ul>
                }.into_any(),
            }}
        </Transition>
    }
}
