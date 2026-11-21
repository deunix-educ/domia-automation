// SPDX-License-Identifier: AGPL-3.0-or-later
//! Commandes d'une camera sur la page live : pilotage PTZ (croix, zoom, vitesses, presets) et bascule jour / nuit (R6 lot 5, ADR 0047).
//! Reserve aux administrateurs (D9).
//!
//! Le pilotage parle au WebSocket `/ws/surveillance/ptz/<uuid>/` (protocole de l'implementation de reference). Une direction **maintenue** renvoie
//! son ordre toutes les 250 ms et envoie `stop` au relachement, a la sortie du bouton et si le pointeur est interrompu ; le serveur
//! arrete de toute facon un mouvement non rafraichi (`PTZ_DEAD_MAN_SECS`).

use dominet_proto::{CameraLiveDto, NightOutcomeDto};
use leptos::prelude::*;
use leptos_fluent::{move_tr, tr};

use crate::camera_fns::camera_night;

/// Un preset, tel que le serveur le liste.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PresetView {
    pub id: i64,
    pub name: String,
}

/// Lit `{"type":"presets","presets":[{id,name,...}]}`.
#[must_use]
pub fn parse_presets(v: &serde_json::Value) -> Vec<PresetView> {
    v["presets"]
        .as_array()
        .map(|list| {
            list.iter()
                .filter_map(|p| {
                    Some(PresetView {
                        id: p["id"].as_i64()?,
                        name: p["name"].as_str()?.to_string(),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Etat du lien avec la camera.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinkState {
    Connecting,
    Ready,
    /// Le serveur a repondu une erreur (message deja lisible).
    Failed(String),
    /// Connexion fermee (camera injoignable, session finie...) : nouvelle tentative en cours.
    Closed,
}

impl LinkState {
    #[must_use]
    pub fn label(&self) -> &'static str {
        match self {
            Self::Connecting => "connecting",
            Self::Ready => "ready",
            Self::Failed(_) => "error",
            Self::Closed => "closed",
        }
    }
}

// ------------------------------------------------------------------ navigateur

#[cfg(feature = "hydrate")]
mod browser {
    use std::cell::{Cell, RefCell};

    use leptos::prelude::*;
    use wasm_bindgen::{JsCast, closure::Closure};
    use web_sys::{MessageEvent, WebSocket};

    use super::{LinkState, PresetView, parse_presets};

    thread_local! {
        static SOCKET: RefCell<Option<WebSocket>> = const { RefCell::new(None) };
        static HOLD: Cell<Option<i32>> = const { Cell::new(None) };
    }

    pub fn send(message: &str) {
        SOCKET.with(|s| {
            if let Some(ws) = s.borrow().as_ref()
                && ws.ready_state() == WebSocket::OPEN
            {
                let _ = ws.send_with_str(message);
            }
        });
    }

    pub fn hold_start(message: String) {
        hold_stop_timer();
        send(&message);
        let tick = Closure::<dyn FnMut()>::new(move || send(&message));
        if let Some(w) = web_sys::window()
            && let Ok(id) = w.set_interval_with_callback_and_timeout_and_arguments_0(
                tick.as_ref().unchecked_ref(),
                250,
            )
        {
            HOLD.with(|h| h.set(Some(id)));
        }
        tick.forget();
    }

    fn hold_stop_timer() {
        if let Some(id) = HOLD.with(Cell::take)
            && let Some(w) = web_sys::window()
        {
            w.clear_interval_with_handle(id);
        }
    }

    pub fn hold_stop() {
        // Seulement si un ordre etait maintenu : un `pointerleave` sans appui n'envoie rien.
        if HOLD.with(|h| h.get().is_some()) {
            hold_stop_timer();
            send(r#"{"action":"stop"}"#);
        }
    }

    /// Ouvre le WebSocket de pilotage ; se rouvre apres une fermeture tant que `stopped` est faux.
    pub fn connect(
        uuid: String,
        link: RwSignal<LinkState>,
        presets: RwSignal<Vec<PresetView>>,
        home_set: RwSignal<bool>,
        stopped: RwSignal<bool>,
    ) {
        if stopped.get_untracked() {
            return;
        }
        let Some(location) = web_sys::window().map(|w| w.location()) else {
            return;
        };
        let scheme = if location.protocol().unwrap_or_default() == "https:" {
            "wss"
        } else {
            "ws"
        };
        let url = format!(
            "{scheme}://{}/ws/surveillance/ptz/{uuid}/",
            location.host().unwrap_or_default()
        );
        link.set(LinkState::Connecting);
        let Ok(ws) = WebSocket::new(&url) else { return };

        let on_message = Closure::<dyn FnMut(MessageEvent)>::new(move |ev: MessageEvent| {
            let Some(text) = ev.data().as_string() else {
                return;
            };
            let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) else {
                return;
            };
            match v["type"].as_str() {
                Some("ready") => {
                    link.set(LinkState::Ready);
                    send(r#"{"action":"get_presets"}"#);
                }
                Some("presets") => presets.set(parse_presets(&v)),
                Some("home_set") => home_set.set(true),
                Some("error") => link.set(LinkState::Failed(
                    v["message"].as_str().unwrap_or("").to_string(),
                )),
                _ => {}
            }
        });
        ws.set_onmessage(Some(on_message.as_ref().unchecked_ref()));
        on_message.forget();

        let uuid_again = uuid.clone();
        let on_close = Closure::<dyn FnMut()>::new(move || {
            hold_stop_timer();
            if stopped.get_untracked() {
                return;
            }
            if !matches!(link.get_untracked(), LinkState::Failed(_)) {
                link.set(LinkState::Closed);
            }
            let again = uuid_again.clone();
            let retry =
                Closure::once_into_js(move || connect(again, link, presets, home_set, stopped));
            if let Some(w) = web_sys::window() {
                let _ = w.set_timeout_with_callback_and_timeout_and_arguments_0(
                    retry.unchecked_ref(),
                    1_500,
                );
            }
        });
        ws.set_onclose(Some(on_close.as_ref().unchecked_ref()));
        on_close.forget();
        SOCKET.with(|s| *s.borrow_mut() = Some(ws));
    }

    pub fn disconnect() {
        hold_stop_timer();
        SOCKET.with(|s| {
            if let Some(ws) = s.borrow_mut().take() {
                let _ = ws.close();
            }
        });
    }
}

/// Envoie un ordre au WebSocket de pilotage (sans effet hors navigateur).
fn ptz_send(message: &str) {
    #[cfg(feature = "hydrate")]
    browser::send(message);
    #[cfg(not(feature = "hydrate"))]
    let _ = message;
}

fn hold_start(message: String) {
    #[cfg(feature = "hydrate")]
    browser::hold_start(message);
    #[cfg(not(feature = "hydrate"))]
    let _ = message;
}

fn hold_stop() {
    #[cfg(feature = "hydrate")]
    browser::hold_stop();
}

// ------------------------------------------------------------------ vues

/// Une direction maintenue : ordre repete tant que le bouton est enfonce.
fn hold_button(
    id: &'static str,
    label_key: &'static str,
    symbol: &'static str,
    message: impl Fn() -> String + Send + Sync + 'static,
) -> impl IntoView {
    let i18n = expect_context::<leptos_fluent::I18n>();
    view! {
        <button
            type="button"
            id=id
            class="vc-btn vc-btn--ghost vc-ptz-btn"
            aria-label=move || i18n.tr(label_key)
            on:pointerdown=move |_| hold_start(message())
            on:pointerup=move |_| hold_stop()
            on:pointerleave=move |_| hold_stop()
            on:pointercancel=move |_| hold_stop()
        >
            {symbol}
        </button>
    }
}

/// Pilotage PTZ d'une camera.
#[component]
fn PtzPad(uuid: String) -> impl IntoView {
    let link = RwSignal::new(LinkState::Connecting);
    let presets = RwSignal::new(Vec::<PresetView>::new());
    let home_set = RwSignal::new(false);
    let stopped = RwSignal::new(false);
    let speed_pt = RwSignal::new(50u32);
    let speed_z = RwSignal::new(50u32);
    let name = RwSignal::new(String::new());
    on_cleanup(move || {
        stopped.set(true);
        #[cfg(feature = "hydrate")]
        browser::disconnect();
    });
    #[cfg(feature = "hydrate")]
    {
        let uuid = uuid.clone();
        Effect::new(move |_| browser::connect(uuid.clone(), link, presets, home_set, stopped));
    }
    #[cfg(not(feature = "hydrate"))]
    let _ = &uuid;

    let move_msg = move |pan: i32, tilt: i32| {
        move || {
            format!(
                r#"{{"action":"move","pan":{pan},"tilt":{tilt},"speed_pt":{}}}"#,
                f64::from(speed_pt.get_untracked()) / 100.0
            )
        }
    };
    let zoom_msg = move |z: i32| {
        move || {
            format!(
                r#"{{"action":"zoom","z":{z},"speed_z":{}}}"#,
                f64::from(speed_z.get_untracked()) / 100.0
            )
        }
    };
    let status = move || match link.get() {
        LinkState::Connecting => tr!("ptz-connecting"),
        LinkState::Ready => tr!("ptz-ready"),
        LinkState::Closed => tr!("ptz-closed"),
        LinkState::Failed(m) => format!("{} {m}", tr!("ptz-error")),
    };
    let save = move || {
        let n = name.get_untracked().trim().to_string();
        if !n.is_empty() {
            ptz_send(&serde_json::json!({"action": "set_preset", "name": n}).to_string());
            name.set(String::new());
        }
    };
    view! {
        <section id="ptz-panel" class="vc-card vc-ptz" data-ptz-state=move || link.get().label()>
            <h3 class="vc-card-title">{move_tr!("ptz-title")}</h3>
            <p id="ptz-status" class="vc-muted" role="status">{status}</p>
            <div class="vc-ptz-pad" role="group" aria-label=move || tr!("ptz-title")>
                <span></span>
                {hold_button("ptz-up", tk!("ptz-up"), "▲", move_msg(0, 1))}
                <span></span>
                {hold_button("ptz-left", tk!("ptz-left"), "◀", move_msg(-1, 0))}
                <button type="button" id="ptz-home" class="vc-btn vc-ptz-btn" aria-label=move || tr!("ptz-home") title=move || tr!("ptz-home")
                    on:click=move |_| ptz_send(r#"{"action":"home"}"#)>"⌂"</button>
                {hold_button("ptz-right", tk!("ptz-right"), "▶", move_msg(1, 0))}
                <span></span>
                {hold_button("ptz-down", tk!("ptz-down"), "▼", move_msg(0, -1))}
                <span></span>
            </div>
            <div class="vc-ptz-zoom">
                {hold_button("ptz-zoom-out", tk!("ptz-zoom-out"), "－", zoom_msg(-1))}
                <button type="button" id="ptz-stop" class="vc-btn vc-ptz-btn" aria-label=move || tr!("ptz-stop") title=move || tr!("ptz-stop")
                    on:click=move |_| ptz_send(r#"{"action":"stop"}"#)>"■"</button>
                {hold_button("ptz-zoom-in", tk!("ptz-zoom-in"), "＋", zoom_msg(1))}
            </div>
            <div class="vc-form-group">
                <label for="ptz-speed-pt">{move_tr!("ptz-speed-pt")}" "{move || format!("{} %", speed_pt.get())}</label>
                <input id="ptz-speed-pt" type="range" min="5" max="100" prop:value=move || speed_pt.get()
                    on:input=move |e| speed_pt.set(event_target_value(&e).parse().unwrap_or(50))/>
            </div>
            <div class="vc-form-group">
                <label for="ptz-speed-z">{move_tr!("ptz-speed-z")}" "{move || format!("{} %", speed_z.get())}</label>
                <input id="ptz-speed-z" type="range" min="5" max="100" prop:value=move || speed_z.get()
                    on:input=move |e| speed_z.set(event_target_value(&e).parse().unwrap_or(50))/>
            </div>
            <p class="vc-btn-row">
                <button type="button" id="ptz-home-save" class="vc-btn vc-btn--ghost vc-btn--sm"
                    on:click=move |_| { home_set.set(false); ptz_send(r#"{"action":"set_home"}"#); }>
                    {move_tr!("ptz-home-save")}
                </button>
                {move || home_set.get().then(|| view! { <span id="ptz-home-saved" class="vc-muted" role="status">{move_tr!("ptz-home-saved")}</span> })}
            </p>
            <h4 class="vc-ptz-subtitle">{move_tr!("ptz-presets")}</h4>
            {move || presets.get().is_empty().then(|| view! { <p id="ptz-no-presets" class="vc-muted">{move_tr!("ptz-no-presets")}</p> })}
            <ul id="ptz-presets" class="vc-list">
                <For each=move || presets.get() key=|p| (p.id, p.name.clone()) let:p>
                    <li data-preset=p.id.to_string()>
                        <span>{p.name.clone()}</span>
                        <button type="button" class="vc-btn vc-btn--ghost vc-btn--sm ptz-goto"
                            on:click=move |_| ptz_send(&format!(r#"{{"action":"goto_preset","id":{}}}"#, p.id))>
                            {move_tr!("ptz-preset-go")}
                        </button>
                        <button type="button" class="vc-btn vc-btn--danger vc-btn--sm ptz-remove"
                            on:click=move |_| ptz_send(&format!(r#"{{"action":"remove_preset","id":{}}}"#, p.id))>
                            {move_tr!("ptz-preset-remove")}
                        </button>
                    </li>
                </For>
            </ul>
            <div class="vc-form-group">
                <label for="ptz-preset-name">{move_tr!("ptz-preset-name")}</label>
                <input id="ptz-preset-name" type="text" maxlength="128" prop:value=move || name.get()
                    on:input=move |e| name.set(event_target_value(&e))
                    on:keydown=move |e| { if e.key() == "Enter" { e.prevent_default(); save(); } }/>
            </div>
            <button type="button" id="ptz-preset-save" class="vc-btn" on:click=move |_| save()>{move_tr!("ptz-preset-save")}</button>
        </section>
    }
}

fn night_message(outcome: &NightOutcomeDto) -> (bool, String) {
    match outcome {
        NightOutcomeDto::Done { action, details } => {
            // `details` est le JSON de la reference : on montre ce qui a change (mode, valeur, luminosite...).
            let v: serde_json::Value = serde_json::from_str(details).unwrap_or_default();
            let shown = ["mode", "value", "brightness", "control"]
                .iter()
                .find_map(|k| {
                    v.get(*k)
                        .map(|x| x.to_string().trim_matches('"').to_string())
                })
                .unwrap_or_default();
            (
                true,
                format!("{} {action} {shown}", tr!("night-done"))
                    .trim()
                    .to_string(),
            )
        }
        NightOutcomeDto::NotConfigured => (false, tr!("night-not-configured")),
        NightOutcomeDto::NotFound => (false, tr!("live-not-found")),
        NightOutcomeDto::InvalidAction => (false, tr!("night-invalid")),
        NightOutcomeDto::Failed(m) => (false, format!("{} {m}", tr!("night-failed"))),
    }
}

/// Boutons jour / nuit, IR-cut et lumiere.
#[component]
fn NightPad(uuid: String) -> impl IntoView {
    let feedback = RwSignal::new(None::<(bool, String)>);
    let run = Action::new(move |action: &&'static str| {
        let (uuid, action) = (uuid.clone(), (*action).to_string());
        async move {
            let out = camera_night(uuid, action)
                .await
                .map_or_else(|_| (false, tr!("crud-load-error")), |o| night_message(&o));
            feedback.set(Some(out));
        }
    });
    let i18n = expect_context::<leptos_fluent::I18n>();
    let button = move |id: &'static str, action: &'static str, key: &'static str| {
        view! {
            <button type="button" id=id class="vc-btn vc-btn--ghost night-btn" on:click=move |_| { run.dispatch(action); }>
                {move || i18n.tr(key)}
            </button>
        }
    };
    view! {
        <section id="night-panel" class="vc-card vc-night">
            <h3 class="vc-card-title">{move_tr!("night-title")}</h3>
            <p class="vc-btn-row">
                {button("night-daynight", "daynight", tk!("night-daynight"))}
                {button("night-ircut", "ircut", tk!("night-ircut"))}
                {button("night-light", "light", tk!("night-light"))}
            </p>
            {move || feedback.get().map(|(ok, text)| {
                let class = if ok { "vc-alert vc-alert--success" } else { "vc-alert vc-alert--error" };
                view! { <div id="night-feedback" class=class role="status">{text}</div> }
            })}
        </section>
    }
}

/// Les commandes d'une camera (administrateurs) : PTZ si elle en a, jour / nuit si une methode est configuree.
#[component]
pub fn CameraControls(camera: CameraLiveDto) -> impl IntoView {
    view! {
        <div class="vc-controls">
            {camera.ptz.then(|| view! { <PtzPad uuid=camera.uuid.clone()/> })}
            {camera.night.then(|| view! { <NightPad uuid=camera.uuid.clone()/> })}
        </div>
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn presets_are_read_and_malformed_entries_are_skipped() {
        let v = json!({"type": "presets", "presets": [
            {"id": 1, "name": "Porte", "token": "p1"},
            {"id": "x", "name": "Sans numero"},
            {"id": 2},
            {"id": 3, "name": "Jardin", "token": ""},
        ]});
        assert_eq!(
            parse_presets(&v),
            [
                PresetView {
                    id: 1,
                    name: "Porte".into()
                },
                PresetView {
                    id: 3,
                    name: "Jardin".into()
                }
            ]
        );
        assert_eq!(parse_presets(&json!({"type": "presets"})).len(), 0);
    }

    #[test]
    fn the_link_states_have_distinct_labels() {
        let labels = [
            LinkState::Connecting.label(),
            LinkState::Ready.label(),
            LinkState::Failed(String::new()).label(),
            LinkState::Closed.label(),
        ];
        let unique: std::collections::BTreeSet<_> = labels.iter().collect();
        assert_eq!(unique.len(), labels.len());
    }
}
