// SPDX-License-Identifier: AGPL-3.0-or-later
//! Enregistrements d'une camera : `/surveillance/recordings/<uuid>` (R6 lot 7, ADR 0049). Tout compte connecte regarde ; les
//! administrateurs suppriment et pilotent l'enregistrement manuel. La lecture est progressive : un `<video>` sur le flux
//! `/surveillance/recordings/<uuid>/<id>/stream/` (requetes `Range`).

use dominet_proto::{DeleteOutcome, ManualRecordOutcome, RecordingDto};
use leptos::prelude::*;
use leptos_fluent::{move_tr, tr};
use leptos_router::components::A;
use leptos_router::hooks::use_params_map;

use crate::camera_fns::{delete_recording, list_recordings, set_manual_record};
use crate::guard::RequireAuth;

#[component]
pub fn RecordingsPage() -> impl IntoView {
    view! {
        <RequireAuth>
            <main class="vc-container">
                <Recordings/>
            </main>
        </RequireAuth>
    }
}

/// `2026-11-08T12:30:05+01:00` -> `12:30:05`.
fn clock(rfc3339: &str) -> String {
    rfc3339.get(11..19).unwrap_or(rfc3339).to_string()
}

fn duration_text(ms: Option<i64>) -> String {
    match ms {
        Some(ms) if ms >= 0 => {
            let s = ms / 1000;
            format!("{}:{:02}", s / 60, s % 60)
        }
        _ => String::new(),
    }
}

fn size_text(bytes: Option<i64>) -> String {
    match bytes {
        // Affichage seulement : la perte de precision de la conversion n'a pas d'importance.
        #[allow(clippy::cast_precision_loss, reason = "affichage en Mio")]
        Some(b) if b >= 0 => format!("{:.1} Mo", b as f64 / 1_048_576.0),
        _ => String::new(),
    }
}

fn type_key(event: &str) -> &'static str {
    match event {
        "motion" => tk!("recordings-type-motion"),
        "manual" => tk!("recordings-type-manual"),
        _ => tk!("recordings-type-continuous"),
    }
}

fn status_key(status: &str) -> &'static str {
    match status {
        "done" => tk!("recordings-status-done"),
        "uploading" => tk!("recordings-status-uploading"),
        "error" => tk!("recordings-status-error"),
        _ => tk!("recordings-status-pending"),
    }
}

#[component]
fn Recordings() -> impl IntoView {
    let params = use_params_map();
    let uuid = move || params.with(|p| p.get("uuid").unwrap_or_default());
    let admin = crate::dashboard::is_admin();
    let day = RwSignal::new(String::new());
    let version = RwSignal::new(0u32);
    let playing = RwSignal::new(None::<i64>);
    let feedback = RwSignal::new(None::<String>);
    // Les segments se terminent tout seuls : la liste se rafraichit (navigateur seulement).
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
        move || (uuid(), day.get(), version.get(), tick.get()),
        |(uuid, day, _, _)| async move { list_recordings(uuid, day).await.map_err(|_| ()) },
    );
    let remove = Action::new(move |id: &i64| {
        let (id, uuid) = (*id, uuid());
        async move {
            let _ = delete_recording(uuid, id)
                .await
                .map(|o| o == DeleteOutcome::Deleted);
            playing.update(|p| {
                if *p == Some(id) {
                    *p = None;
                }
            });
            version.update(|v| *v += 1);
        }
    });
    let toggle = Action::new(move |on: &bool| {
        let (on, uuid) = (*on, uuid());
        async move {
            let text = match set_manual_record(uuid, on).await {
                Ok(ManualRecordOutcome::Done { .. }) => None,
                Ok(ManualRecordOutcome::NotManual) => Some(tr!("recordings-not-manual")),
                Ok(ManualRecordOutcome::NotFound) => Some(tr!("live-not-found")),
                Ok(ManualRecordOutcome::Unavailable(m)) => {
                    Some(format!("{} {m}", tr!("recordings-unavailable")))
                }
                Err(_) => Some(tr!("crud-load-error")),
            };
            feedback.set(text);
            version.update(|v| *v += 1);
        }
    });
    let shift = move |delta: i64| {
        move |_| {
            // Jour precedent / suivant a partir de la date choisie (ou du jour affiche) : calcul sur les chiffres, sans fuseau.
            let current = day.get_untracked();
            if let Some(next) = shift_day(&current, delta) {
                day.set(next);
            }
        }
    };
    // La liste se recharge toute seule : le lecteur et le selecteur de jour restent HORS de ce qui est recree, sinon la video
    // repartirait du debut a chaque rechargement.
    view! {
        <Transition fallback=|| ()>
            {move || match list.get() {
                None => ().into_any(),
                Some(Err(())) => view! { <div class="vc-alert vc-alert--error" role="alert" id="recordings-error">{move_tr!("crud-load-error")}</div> }.into_any(),
                Some(Ok(None)) => view! { <p id="recordings-not-found">{move_tr!("live-not-found")}</p> }.into_any(),
                Some(Ok(Some(data))) => {
                    let name = data.camera_name.clone();
                    view! { <div class="vc-page-header"><h2>{move_tr!("recordings-title")}" \u{b7} "{name}</h2></div> }.into_any()
                }
            }}
        </Transition>
        <p class="vc-btn-row">
            <A href="/surveillance" attr:class="vc-btn vc-btn--ghost">{move_tr!("live-back")}</A>
            <A href=move || format!("/surveillance/live/{}", uuid()) attr:class="vc-btn vc-btn--ghost">{move_tr!("live-title")}</A>
        </p>
        <div class="vc-card">
            <div class="vc-snap-filters">
                <div class="vc-form-group">
                    <label for="rec-day">{move_tr!("snapshots-day")}</label>
                    <input id="rec-day" type="date" prop:value=move || day.get() on:change=move |e| day.set(event_target_value(&e))/>
                </div>
                <p class="vc-btn-row">
                    <button type="button" id="rec-prev" class="vc-btn vc-btn--ghost vc-btn--sm" on:click=shift(-1)>{move_tr!("recordings-prev")}</button>
                    <button type="button" id="rec-next" class="vc-btn vc-btn--ghost vc-btn--sm" on:click=shift(1)>{move_tr!("recordings-next")}</button>
                </p>
            </div>
            <Transition fallback=|| ()>
                {move || match list.get() {
                    Some(Ok(Some(data))) if data.recording_mode == "manual" && admin.get() => {
                        let manual_on = data.manual_on;
                        view! {
                            <p class="vc-btn-row">
                                <span id="rec-manual-state" class="vc-muted">
                                    {match manual_on {
                                        Some(true) => tr!("recordings-manual-on"),
                                        Some(false) => tr!("recordings-manual-off"),
                                        None => tr!("recordings-manual-unknown"),
                                    }}
                                </span>
                                <button type="button" id="rec-manual-start" class="vc-btn vc-btn--sm" on:click=move |_| { toggle.dispatch(true); }>{move_tr!("recordings-manual-start")}</button>
                                <button type="button" id="rec-manual-stop" class="vc-btn vc-btn--ghost vc-btn--sm" on:click=move |_| { toggle.dispatch(false); }>{move_tr!("recordings-manual-stop")}</button>
                            </p>
                        }.into_any()
                    }
                    _ => ().into_any(),
                }}
            </Transition>
            {move || feedback.get().map(|t| view! { <div id="rec-feedback" class="vc-alert vc-alert--error" role="alert">{t}</div> })}
        </div>
        {move || playing.get().map(|id| view! {
            <div class="vc-card vc-rec-player">
                <video id="rec-player" controls=true autoplay=true preload="metadata" src=format!("/surveillance/recordings/{}/{id}/stream/", uuid())></video>
            </div>
        })}
        <Transition fallback=|| ()>
            {move || match list.get() {
                Some(Ok(Some(data))) if data.items.is_empty() => view! { <p id="recordings-empty">{move_tr!("recordings-empty")}</p> }.into_any(),
                Some(Ok(Some(data))) => view! {
                    <table class="vc-table" id="recordings-table">
                        <thead><tr>
                            <th>{move_tr!("recordings-col-time")}</th>
                            <th>{move_tr!("recordings-col-duration")}</th>
                            <th>{move_tr!("recordings-col-type")}</th>
                            <th>{move_tr!("recordings-col-status")}</th>
                            <th>{move_tr!("recordings-col-size")}</th>
                            <th></th>
                        </tr></thead>
                        <tbody>
                            {data.items.into_iter().map(|r| row(r, playing, remove, admin)).collect_view()}
                        </tbody>
                    </table>
                }.into_any(),
                _ => ().into_any(),
            }}
        </Transition>
    }
}

fn row(
    r: RecordingDto,
    playing: RwSignal<Option<i64>>,
    remove: Action<i64, ()>,
    admin: Memo<bool>,
) -> impl IntoView {
    let i18n = expect_context::<leptos_fluent::I18n>();
    let (id, type_k, status_k) = (r.id, type_key(&r.event_type), status_key(&r.status));
    let (error, status, event) = (r.error, r.status, r.event_type);
    view! {
        <tr data-recording=id.to_string() data-status=status data-type=event>
            <td>{clock(&r.started_at)}</td>
            <td>{duration_text(r.duration_ms)}</td>
            <td>{move || i18n.tr(type_k)}</td>
            <td title=error>{move || i18n.tr(status_k)}</td>
            <td>{size_text(r.size_bytes)}</td>
            <td>
                <button type="button" class="vc-btn vc-btn--ghost vc-btn--sm rec-play" on:click=move |_| playing.set(Some(id))>{move_tr!("recordings-play")}</button>
                {move || admin.get().then(|| view! {
                    <button type="button" class="vc-btn vc-btn--danger vc-btn--sm rec-delete" on:click=move |_| { remove.dispatch(id); }>{move_tr!("ptz-preset-remove")}</button>
                })}
            </td>
        </tr>
    }
}

/// Le jour `AAAA-MM-JJ` decale de `delta` jours ; sans date valide, on part d'aujourd'hui cote serveur (jour vide) : pas de decalage.
fn shift_day(day: &str, delta: i64) -> Option<String> {
    let mut parts = day.split('-').map(str::parse::<i64>);
    let (y, m, d) = (
        parts.next()?.ok()?,
        parts.next()?.ok()?,
        parts.next()?.ok()?,
    );
    let days = days_from_civil(y, m, d) + delta;
    let (y, m, d) = civil_from_days(days);
    Some(format!("{y:04}-{m:02}-{d:02}"))
}

// Algorithmes de H. Hinnant (calendrier gregorien proleptique), sans dependance.
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * (m + if m > 2 { -3 } else { 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (if m <= 2 { y + 1 } else { y }, m, d)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn days_move_across_months_years_and_leap_days() {
        assert_eq!(shift_day("2026-11-08", 1).as_deref(), Some("2026-11-09"));
        assert_eq!(shift_day("2026-11-30", 1).as_deref(), Some("2026-12-01"));
        assert_eq!(shift_day("2026-01-01", -1).as_deref(), Some("2025-12-31"));
        assert_eq!(shift_day("2028-02-28", 1).as_deref(), Some("2028-02-29"));
        assert_eq!(shift_day("2027-02-28", 1).as_deref(), Some("2027-03-01"));
        assert_eq!(shift_day("", 1), None);
        assert_eq!(shift_day("pas-un-jour", 1), None);
    }

    #[test]
    fn texts_are_short_and_stable() {
        assert_eq!(clock("2026-11-08T12:30:05+01:00"), "12:30:05");
        assert_eq!(duration_text(Some(61_500)), "1:01");
        assert_eq!(duration_text(None), "");
        assert_eq!(size_text(Some(5 * 1_048_576)), "5.0 Mo");
    }
}
