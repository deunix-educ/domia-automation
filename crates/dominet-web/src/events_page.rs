// SPDX-License-Identifier: AGPL-3.0-or-later
//! Camera events: `/surveillance/events` (R6 lot 8, ADR 0050) — recent motions and object detections, with the annotated image
//! when it exists. Any logged-in account views; administrators set the detector (on or off, keeps the images).

use leptos::prelude::*;
use leptos_fluent::{move_tr, tr};
use leptos_router::components::A;

use crate::camera_fns::{get_detector, list_camera_cards, list_camera_events, set_detector};
use crate::guard::RequireAuth;

#[component]
pub fn EventsPage() -> impl IntoView {
    view! {
        <RequireAuth>
            <main class="vc-container">
                <Events/>
            </main>
        </RequireAuth>
    }
}

/// `2026-11-08T12:30:05+01:00` -> `2026-11-08 12:30:05`.
fn when(rfc3339: &str) -> String {
    rfc3339.get(..19).unwrap_or(rfc3339).replace('T', " ")
}

#[component]
fn DetectorCard() -> impl IntoView {
    let enabled = RwSignal::new(false);
    let save_frames = RwSignal::new(false);
    let saved = RwSignal::new(false);
    let current = Resource::new(|| (), |()| async { get_detector().await.ok() });
    Effect::new(move |_| {
        if let Some(Some(d)) = current.get() {
            enabled.set(d.enabled);
            save_frames.set(d.save_frames);
        }
    });
    let save = Action::new(move |(): &()| {
        let (e, s) = (enabled.get_untracked(), save_frames.get_untracked());
        async move {
            saved.set(set_detector(e, s).await.is_ok());
        }
    });
    view! {
        <section class="vc-card" id="detector-card">
            <h3 class="vc-card-title">{move_tr!("detector-title")}</h3>
            <div class="vc-form-group vc-form-group--check">
                <label for="detector-enabled">{move_tr!("detector-enabled")}</label>
                <input id="detector-enabled" type="checkbox" prop:checked=move || enabled.get()
                    on:change=move |e| { saved.set(false); enabled.set(event_target_checked(&e)); }/>
                <div class="vc-hint">{move_tr!("detector-enabled-hint")}</div>
            </div>
            <div class="vc-form-group vc-form-group--check">
                <label for="detector-frames">{move_tr!("detector-frames")}</label>
                <input id="detector-frames" type="checkbox" prop:checked=move || save_frames.get()
                    on:change=move |e| { saved.set(false); save_frames.set(event_target_checked(&e)); }/>
            </div>
            <p class="vc-btn-row">
                <button type="button" id="detector-save" class="vc-btn vc-btn--sm" on:click=move |_| { save.dispatch(()); }>{move_tr!("crud-save")}</button>
                {move || saved.get().then(|| view! { <span id="detector-saved" class="vc-muted" role="status">{move_tr!("detector-saved")}</span> })}
            </p>
        </section>
    }
}

#[component]
fn Events() -> impl IntoView {
    let admin = crate::dashboard::is_admin();
    let camera = RwSignal::new(String::new());
    let cameras = Resource::new(
        || (),
        |()| async { list_camera_cards().await.unwrap_or_default() },
    );
    // The events arrive without user action: the list refreshes by itself (browser only).
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
    let list = Resource::new(
        move || (camera.get(), tick.get()),
        |(camera, _)| async move { list_camera_events(camera).await.map_err(|_| ()) },
    );
    view! {
        <div class="vc-page-header"><h2>{move_tr!("events-title")}</h2></div>
        <p class="vc-btn-row"><A href="/surveillance" attr:class="vc-btn vc-btn--ghost">{move_tr!("live-back")}</A></p>
        {move || admin.get().then(|| view! { <DetectorCard/> })}
        <div class="vc-card">
            <div class="vc-form-group">
                <label for="events-camera">{move_tr!("snapshots-camera")}</label>
                <select id="events-camera" on:change=move |e| camera.set(event_target_value(&e))>
                    <option value="">{move_tr!("snapshots-all-cameras")}</option>
                    <Transition fallback=|| ()>
                        {move || cameras.get().unwrap_or_default().into_iter().map(|c| view! { <option value=c.uuid.clone()>{c.name.clone()}</option> }).collect_view()}
                    </Transition>
                </select>
            </div>
        </div>
        <Transition fallback=|| ()>
            {move || match list.get() {
                None => ().into_any(),
                Some(Err(())) => view! { <div class="vc-alert vc-alert--error" role="alert" id="events-error">{move_tr!("crud-load-error")}</div> }.into_any(),
                Some(Ok(items)) if items.is_empty() => view! { <p id="events-empty">{move_tr!("events-empty")}</p> }.into_any(),
                Some(Ok(items)) => view! {
                    <div class="vc-table-wrap">
                    <table class="vc-table" id="events-table">
                        <thead><tr>
                            <th>{move_tr!("events-col-time")}</th>
                            <th>{move_tr!("snapshots-camera")}</th>
                            <th>{move_tr!("events-col-kind")}</th>
                            <th>{move_tr!("events-col-detail")}</th>
                        </tr></thead>
                        <tbody>
                            {items.into_iter().map(|e| {
                                let detail = match (&e.motion_pct, e.classes.is_empty()) {
                                    (Some(p), _) => format!("{p:.1} %"),
                                    (None, _) => e.classes.clone(),
                                };
                                let kind = if e.kind == "motion" { tr!("events-motion") } else { tr!("events-detection") };
                                let detail_title = detail.clone();
                                let camera_title = e.camera_name.clone();
                                let frame = e.frame_id.map(|id| format!("/surveillance/detections/{id}/frame/"));
                                view! {
                                    <tr data-kind=e.kind.clone()>
                                        <td>{when(&e.at)}</td>
                                        <td title=camera_title>{e.camera_name.clone()}</td>
                                        <td>{kind}</td>
                                        <td title=detail_title>
                                            {detail}
                                            {frame.map(|href| view! { " " <a href=href target="_blank" rel="noopener" class="event-frame">{move_tr!("events-frame")}</a> })}
                                        </td>
                                    </tr>
                                }
                            }).collect_view()}
                        </tbody>
                    </table>
                    </div>
                }.into_any(),
            }}
        </Transition>
    }
}

#[cfg(test)]
mod tests {
    use super::when;

    #[test]
    fn the_time_is_shown_without_zone_or_t() {
        assert_eq!(when("2026-11-08T12:30:05+01:00"), "2026-11-08 12:30:05");
        assert_eq!(when("court"), "court");
    }
}
