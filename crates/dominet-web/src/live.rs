// SPDX-License-Identifier: AGPL-3.0-or-later
//! Live d'une camera : lecteur WHEP et page `/surveillance/live/<uuid>` (R6 lot 4, ADR 0046). Le lecteur sert aussi au widget camera du
//! tableau de bord. Tout compte connecte peut regarder ; le flux vient de MediaMTX, jamais de la camera.

use dominet_proto::CameraLiveDto;
use leptos::prelude::*;
use leptos_fluent::move_tr;
use leptos_router::components::A;
use leptos_router::hooks::use_params_map;

use crate::camera_fns::{camera_live, capture_snapshot};
use crate::guard::RequireAuth;
use crate::ptz_panel::CameraControls;
use crate::whep::{Failure, StatsSample, video_id};

/// Ce que le lecteur montre.
#[derive(Debug, Clone, PartialEq)]
pub enum PlayerState {
    /// Negociation en cours, ou attente de la premiere image.
    Connecting,
    Live,
    /// Session perdue ou figee : nouvelle tentative en cours.
    Reconnecting,
    Failed(Failure),
}

impl PlayerState {
    /// Valeur de `data-live-state` (lue par les tests de bout en bout).
    #[must_use]
    pub fn label(&self) -> &'static str {
        match self {
            Self::Connecting => "connecting",
            Self::Live => "live",
            Self::Reconnecting => "reconnecting",
            Self::Failed(Failure::NoH264) => "no-h264",
            Self::Failed(Failure::NoStream) => "no-stream",
            Self::Failed(Failure::Http(_)) => "http-error",
            Self::Failed(Failure::Browser(_)) => "browser-error",
        }
    }
}

fn message(state: &PlayerState) -> Option<String> {
    use leptos_fluent::tr;
    Some(match state {
        PlayerState::Live => return None,
        PlayerState::Connecting => tr!("live-connecting"),
        PlayerState::Reconnecting => tr!("live-reconnecting"),
        PlayerState::Failed(Failure::NoH264) => tr!("live-no-h264"),
        PlayerState::Failed(Failure::NoStream) => tr!("live-no-stream"),
        PlayerState::Failed(Failure::Http(code)) => tr!("live-http", {"code" => code.to_string()}),
        PlayerState::Failed(Failure::Browser(m)) => tr!("live-browser", {"message" => m.clone()}),
    })
}

fn stats_line(s: &StatsSample) -> String {
    let rtt = s
        .rtt_ms
        .map_or_else(String::new, |r| format!(" · RTT {r:.0} ms"));
    format!("{:.0}×{:.0} · {:.0} i/s{rtt}", s.width, s.height, s.fps)
}

/// Lecteur d'une camera. `instance` distingue deux affichages de la meme camera dans une page (identifiant du `<video>`).
#[component]
pub fn WhepPlayer(camera: CameraLiveDto, #[prop(default = 0)] instance: u32) -> impl IntoView {
    let id = video_id(&camera.uuid, instance);
    let state = RwSignal::new(PlayerState::Connecting);
    let metrics = RwSignal::new(None::<StatsSample>);
    let stopped = RwSignal::new(false);
    // Navigateur sans H.264 : image fixe rafraichie chaque seconde (les images de la camera, voir ADR 0048) ; le numero change l'adresse.
    let tick = RwSignal::new(0u32);
    on_cleanup(move || stopped.set(true));
    let snapshot_url = format!("/surveillance/camera/{}/snapshot/", camera.uuid);

    #[cfg(feature = "hydrate")]
    {
        let (url, video) = (camera.whep_url.clone(), id.clone());
        Effect::new(move |_| {
            wasm_bindgen_futures::spawn_local(run(
                url.clone(),
                video.clone(),
                state,
                metrics,
                stopped,
                tick,
            ));
        });
    }

    view! {
        <div class="vc-live" data-live-state=move || state.get().label() data-camera=camera.uuid.clone()>
            <video id=id class="vc-live-video" autoplay muted playsinline title=camera.name.clone()></video>
            {move || (state.get() == PlayerState::Failed(Failure::NoH264)).then(|| {
                let url = snapshot_url.clone();
                view! { <img class="vc-live-video vc-live-fallback" alt="" src=move || format!("{url}?t={}", tick.get())/> }
            })}
            {move || message(&state.get()).map(|m| {
                let class = match state.get() {
                    PlayerState::Failed(Failure::NoH264) => "vc-live-message vc-live-message--notice",
                    PlayerState::Failed(_) => "vc-live-message vc-live-message--error",
                    _ => "vc-live-message",
                };
                view! { <div class=class role="status">{m}</div> }
            })}
            {move || metrics.get().filter(|_| state.get() == PlayerState::Live).map(|s| {
                view! { <div class="vc-live-stats">{stats_line(&s)}</div> }
            })}
        </div>
    }
}

/// Boucle du lecteur : connexion, surveillance des images decodees, reconnexion avec delai croissant.
#[cfg(feature = "hydrate")]
async fn run(
    url: String,
    video: String,
    state: RwSignal<PlayerState>,
    metrics: RwSignal<Option<StatsSample>>,
    stopped: RwSignal<bool>,
    tick: RwSignal<u32>,
) {
    use crate::realtime::{backoff_ms, with_jitter};
    use crate::whep::browser::{connect, h264_supported, now_ms, sleep_ms};
    use crate::whep::{Health, STATS_EVERY_MS, StallDetector};

    if !h264_supported() {
        state.set(PlayerState::Failed(Failure::NoH264));
        refresh_snapshot(tick, stopped).await;
        return;
    }
    let mut attempt = 0u32;
    while !stopped.get_untracked() {
        state.set(if attempt == 0 {
            PlayerState::Connecting
        } else {
            PlayerState::Reconnecting
        });
        match connect(&url, &video).await {
            Ok(session) => {
                attempt = 0;
                let mut detector = StallDetector::new(now_ms());
                loop {
                    sleep_ms(STATS_EVERY_MS).await;
                    if stopped.get_untracked() || session.is_down() {
                        break;
                    }
                    let sample = session.stats().await;
                    if detector.observe(now_ms(), sample.frames_decoded) == Health::Stalled {
                        break;
                    }
                    if sample.frames_decoded > 0.0 {
                        state.set(PlayerState::Live);
                    }
                    metrics.set(Some(sample));
                }
                session.close().await;
                metrics.set(None);
            }
            Err(Failure::NoH264) => {
                state.set(PlayerState::Failed(Failure::NoH264));
                refresh_snapshot(tick, stopped).await;
                return;
            }
            Err(failure) => state.set(PlayerState::Failed(failure)),
        }
        if stopped.get_untracked() {
            break;
        }
        let delay = with_jitter(backoff_ms(attempt), js_sys::Math::random());
        attempt += 1;
        sleep_ms(i32::try_from(delay).unwrap_or(30_000)).await;
    }
}

/// Affichage de repli : une nouvelle image chaque seconde, tant que la page est ouverte.
#[cfg(feature = "hydrate")]
async fn refresh_snapshot(tick: RwSignal<u32>, stopped: RwSignal<bool>) {
    while !stopped.get_untracked() {
        tick.update(|t| *t = t.wrapping_add(1));
        crate::whep::browser::sleep_ms(1_000).await;
    }
}

/// Page `/surveillance/live/<uuid>`.
#[component]
pub fn LivePage() -> impl IntoView {
    view! {
        <RequireAuth>
            <main class="vc-container">
                <Live/>
            </main>
        </RequireAuth>
    }
}

#[component]
fn Live() -> impl IntoView {
    let params = use_params_map();
    let uuid = move || params.with(|p| p.get("uuid").unwrap_or_default());
    let admin = crate::dashboard::is_admin();
    let camera = Resource::new(uuid, |uuid| async move {
        camera_live(uuid).await.map_err(|_| ())
    });
    view! {
        <Transition fallback=|| ()>
            {move || match camera.get() {
                None => ().into_any(),
                Some(Err(())) => view! { <div class="vc-alert vc-alert--error" role="alert">{move_tr!("crud-load-error")}</div> }.into_any(),
                Some(Ok(None)) => view! {
                    <div class="vc-page-header"><h2>{move_tr!("live-title")}</h2></div>
                    <p id="live-not-found">{move_tr!("live-not-found")}</p>
                    <p><A href="/surveillance" attr:class="vc-btn vc-btn--ghost">{move_tr!("live-back")}</A></p>
                }.into_any(),
                Some(Ok(Some(cam))) => { let snapshot_uuid = cam.uuid.clone(); let controls = cam.clone(); view! {
                    <div class="vc-page-header"><h2>{cam.name.clone()}</h2></div>
                    <p class="vc-btn-row">
                        <A href="/surveillance" attr:class="vc-btn vc-btn--ghost">{move_tr!("live-back")}</A>
                        <A href="/surveillance/snapshots" attr:class="vc-btn vc-btn--ghost">{move_tr!("snapshots-title")}</A>
                        <A href=format!("/surveillance/recordings/{}", cam.uuid) attr:class="vc-btn vc-btn--ghost" attr:id="live-recordings-link">{move_tr!("recordings-title")}</A>
                    </p>
                    <div class="vc-live-layout">
                        <div class="vc-live-main">
                            <div class="vc-card vc-live-card">
                                <WhepPlayer camera=cam.clone()/>
                            </div>
                            {move || admin.get().then(|| view! { <SnapshotButton uuid=snapshot_uuid.clone()/> })}
                        </div>
                        {move || admin.get().then(|| view! { <CameraControls camera=controls.clone()/> })}
                    </div>
                }.into_any() },
            }}
        </Transition>
    }
}

/// Bouton « Photographier » (administrateurs) : inscrit la derniere image de la camera dans la galerie.
#[component]
fn SnapshotButton(uuid: String) -> impl IntoView {
    let result = RwSignal::new(None::<(bool, String)>);
    let capture = Action::new(move |(): &()| {
        let uuid = uuid.clone();
        async move {
            use dominet_proto::CaptureOutcome;
            let out = match capture_snapshot(uuid).await {
                Ok(CaptureOutcome::Saved(_)) => (true, leptos_fluent::tr!("snapshots-saved")),
                Ok(CaptureOutcome::NoImage) => (false, leptos_fluent::tr!("snapshots-no-image")),
                Ok(CaptureOutcome::NotFound) => (false, leptos_fluent::tr!("live-not-found")),
                Err(_) => (false, leptos_fluent::tr!("crud-load-error")),
            };
            result.set(Some(out));
        }
    });
    view! {
        <p class="vc-btn-row">
            <button type="button" id="snapshot-capture" class="vc-btn" on:click=move |_| { capture.dispatch(()); }>
                {move_tr!("snapshots-capture")}
            </button>
        </p>
        {move || result.get().map(|(ok, text)| {
            let class = if ok { "vc-alert vc-alert--success" } else { "vc-alert vc-alert--error" };
            view! { <div id="snapshot-result" class=class role="status">{text}</div> }
        })}
    }
}
