// SPDX-License-Identifier: AGPL-3.0-or-later
//! Temps reel cote navigateur : connexion WebSocket a `/ws/notifications/`, etat de la connexion et notifications a l'ecran.
//!
//! La connexion s'ouvre quand un compte est connecte et se ferme a la deconnexion. Si elle tombe (reseau, redemarrage du
//! serveur), elle est retablie avec un delai croissant (1 s, 2 s, 4 s... 30 s au plus, avec un peu d'aleatoire pour que
//! les onglets ne reviennent pas tous au meme instant). Un code de fermeture `4401` (session terminee) arrete les
//! tentatives. Le navigateur repond `pong` aux battements du serveur.
//!
//! **Abonnements** : un composant qui veut suivre un sujet (l'etat d'un appareil) appelle [`Realtime::subscribe`] et
//! [`Realtime::unsubscribe`] ; les abonnements sont comptes (deux widgets sur le meme appareil = un abonnement). La connexion
//! envoie les differences au serveur et **renvoie tous les abonnements a chaque (re)connexion** (le serveur n'en garde rien).

use std::collections::{HashMap, HashSet};

use dominet_proto::{DeviceStateDto, NotificationLevel, SystemStatsDto, Topic};
use leptos::prelude::*;

/// Delai (ms) avant la tentative numero `attempt` (0 pour la premiere reconnexion) : 1 s doublant jusqu'a 30 s.
#[must_use]
pub fn backoff_ms(attempt: u32) -> u32 {
    1_000u32.saturating_mul(1 << attempt.min(5)).min(30_000)
}

/// Delai de reconnexion avec jusqu'a +25 % d'aleatoire (`unit` dans `0.0..=1.0`) pour desynchroniser les onglets.
#[must_use]
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)] // borne par `clamp` : resultat entre 0 et 7 500
pub fn with_jitter(base_ms: u32, unit: f64) -> u32 {
    base_ms + (unit.clamp(0.0, 1.0) * f64::from(base_ms) * 0.25) as u32
}

/// Adresse du WebSocket d'une page (`http` -> `ws`, `https` -> `wss`) : toujours la meme origine que la page.
#[must_use]
pub fn ws_url(page_protocol: &str, host: &str) -> String {
    let scheme = if page_protocol == "https:" {
        "wss"
    } else {
        "ws"
    };
    format!("{scheme}://{host}/ws/notifications/")
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RealtimeState {
    Connecting,
    Online,
    Offline,
}

impl RealtimeState {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Connecting => "connecting",
            Self::Online => "online",
            Self::Offline => "offline",
        }
    }
}

/// Abonnements demandes par les composants, avec leur nombre de demandeurs. Les notifications ne s'y trouvent pas : le
/// serveur les envoie toujours.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct TopicCounts(HashMap<Topic, usize>);

impl TopicCounts {
    /// Ajoute un demandeur ; vrai si c'est le premier (l'abonnement doit alors etre envoye).
    pub fn add(&mut self, topic: Topic) -> bool {
        let n = self.0.entry(topic).or_insert(0);
        *n += 1;
        *n == 1
    }

    /// Retire un demandeur ; vrai si c'etait le dernier. Retirer un sujet inconnu ne fait rien.
    pub fn remove(&mut self, topic: &Topic) -> bool {
        match self.0.get_mut(topic) {
            Some(n) if *n > 1 => {
                *n -= 1;
                false
            }
            Some(_) => {
                self.0.remove(topic);
                true
            }
            None => false,
        }
    }

    /// Sujets a suivre cote serveur.
    #[must_use]
    pub fn wanted(&self) -> HashSet<Topic> {
        self.0
            .keys()
            .filter(|t| **t != Topic::Notifications)
            .cloned()
            .collect()
    }
}

/// Differences a envoyer au serveur : (sujets a demander, sujets a abandonner).
#[must_use]
pub fn subscription_diff<S: std::hash::BuildHasher>(
    wanted: &HashSet<Topic, S>,
    subscribed: &HashSet<Topic, S>,
) -> (Vec<Topic>, Vec<Topic>) {
    let mut add: Vec<_> = wanted.difference(subscribed).cloned().collect();
    let mut remove: Vec<_> = subscribed.difference(wanted).cloned().collect();
    // Ordre stable (tests, journaux).
    add.sort_by_key(ToString::to_string);
    remove.sort_by_key(ToString::to_string);
    (add, remove)
}

/// Une notification affichee a l'ecran.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Toast {
    pub id: u32,
    pub level: NotificationLevel,
    pub text: String,
}

/// Etat temps reel partage (contexte Leptos).
#[derive(Clone, Copy, Debug)]
pub struct Realtime {
    pub state: RwSignal<RealtimeState>,
    pub toasts: RwSignal<Vec<Toast>>,
    /// Sujets demandes par les composants (voir [`Realtime::subscribe`]).
    pub topics: RwSignal<TopicCounts>,
    /// Dernier etat recu de chaque appareil suivi.
    pub devices: RwSignal<HashMap<i64, DeviceStateDto>>,
    /// Dernieres ressources de la machine recues (sujet `system`).
    pub system: RwSignal<Option<SystemStatsDto>>,
    /// Nombre d'alertes en cours (sujet `alerts`).
    pub alerts: RwSignal<u32>,
    /// Compteur d'evenements Android recus (sujets `android:<id>`) : les pages Android se rechargent quand il change.
    pub android: RwSignal<u32>,
    /// Compteur de progressions d'envoi de messages (sujets `message:<id>`) : les pages de la messagerie se rechargent.
    pub messaging: RwSignal<u32>,
    /// Etat de la decouverte ONVIF (sujet `discovery`, administrateurs).
    pub discovery: RwSignal<dominet_proto::DiscoveryView>,
}

impl Realtime {
    #[must_use]
    pub fn new() -> Self {
        Self {
            state: RwSignal::new(RealtimeState::Connecting),
            toasts: RwSignal::new(vec![]),
            topics: RwSignal::new(TopicCounts::default()),
            devices: RwSignal::new(HashMap::new()),
            system: RwSignal::new(None),
            alerts: RwSignal::new(0),
            android: RwSignal::new(0),
            messaging: RwSignal::new(0),
            discovery: RwSignal::new(dominet_proto::DiscoveryView::default()),
        }
    }

    /// Demande un sujet (compte les demandeurs). Sans effet tant que la connexion n'est pas ouverte : elle l'enverra a
    /// l'ouverture.
    pub fn subscribe(&self, topic: Topic) {
        self.topics.update(|t| {
            t.add(topic);
        });
    }

    /// Abandonne un sujet demande par [`Realtime::subscribe`].
    pub fn unsubscribe(&self, topic: &Topic) {
        self.topics.update(|t| {
            t.remove(topic);
        });
    }

    /// Dernier etat recu de cet appareil (reactif).
    #[must_use]
    pub fn device(&self, id: i64) -> Option<DeviceStateDto> {
        self.devices.with(|m| m.get(&id).cloned())
    }

    pub fn dismiss(&self, id: u32) {
        self.toasts.update(|t| t.retain(|x| x.id != id));
    }
}

impl Default for Realtime {
    fn default() -> Self {
        Self::new()
    }
}

/// Connexion WebSocket du navigateur (feature `hydrate` seulement).
#[cfg(feature = "hydrate")]
pub mod client {
    use std::cell::RefCell;
    use std::collections::{HashMap, HashSet};
    use std::rc::Rc;

    use dominet_proto::{Topic, WsClientMsg, WsServerMsg};
    use leptos::prelude::*;
    use wasm_bindgen::JsCast;
    use wasm_bindgen::closure::Closure;
    use web_sys::{CloseEvent, MessageEvent, WebSocket};

    use super::{
        Realtime, RealtimeState, Toast, backoff_ms, subscription_diff, with_jitter, ws_url,
    };

    /// Code de fermeture envoye par le serveur quand la session n'est plus valide.
    const SESSION_ENDED: u16 = 4401;
    /// Duree d'affichage d'une notification.
    const TOAST_MS: i32 = 8_000;

    struct Inner {
        rt: Realtime,
        socket: Option<WebSocket>,
        /// Fermetures liees au socket courant (elles doivent vivre aussi longtemps que lui).
        callbacks: Vec<Closure<dyn FnMut(JsValueAlias)>>,
        attempt: u32,
        stopped: bool,
        next_toast: u32,
        /// Sujets que le serveur connait pour la connexion courante (vide a chaque ouverture et a chaque coupure).
        subscribed: HashSet<Topic>,
    }

    type JsValueAlias = wasm_bindgen::JsValue;

    /// Poignee sur la connexion ; `stop` la ferme definitivement.
    #[derive(Clone)]
    pub struct Handle(Rc<RefCell<Inner>>);

    impl std::fmt::Debug for Handle {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str("Handle")
        }
    }

    /// Ouvre la connexion (et la maintient jusqu'a `stop`).
    #[must_use]
    pub fn start(rt: Realtime) -> Handle {
        let handle = Handle(Rc::new(RefCell::new(Inner {
            rt,
            socket: None,
            callbacks: vec![],
            attempt: 0,
            stopped: false,
            next_toast: 0,
            subscribed: HashSet::new(),
        })));
        open(&handle);
        handle
    }

    impl Handle {
        pub fn stop(&self) {
            let mut inner = self.0.borrow_mut();
            inner.stopped = true;
            detach(&mut inner);
            inner.subscribed.clear();
            inner.rt.toasts.set(vec![]);
            inner.rt.devices.set(HashMap::default());
            inner.rt.system.set(None);
            inner.rt.alerts.set(0);
            inner.rt.state.set(RealtimeState::Connecting);
        }

        /// Envoie au serveur les abonnements ajoutes ou retires depuis le dernier envoi (rien si la connexion n'est pas
        /// ouverte : `Hello` relancera la synchronisation).
        pub fn sync(&self) {
            let mut inner = self.0.borrow_mut();
            let Some(ws) = inner.socket.clone() else {
                return;
            };
            if ws.ready_state() != WebSocket::OPEN {
                return;
            }
            let wanted = inner.rt.topics.with_untracked(super::TopicCounts::wanted);
            let (add, remove) = subscription_diff(&wanted, &inner.subscribed);
            for topic in add {
                let msg = WsClientMsg::Subscribe {
                    topic: topic.to_string(),
                };
                if serde_json::to_string(&msg).is_ok_and(|m| ws.send_with_str(&m).is_ok()) {
                    inner.subscribed.insert(topic);
                }
            }
            for topic in remove {
                let msg = WsClientMsg::Unsubscribe {
                    topic: topic.to_string(),
                };
                if serde_json::to_string(&msg).is_ok_and(|m| ws.send_with_str(&m).is_ok()) {
                    inner.subscribed.remove(&topic);
                }
            }
        }
    }

    /// Detache les gestionnaires du socket courant puis le ferme (sans declencher de reconnexion).
    fn detach(inner: &mut Inner) {
        if let Some(ws) = inner.socket.take() {
            ws.set_onopen(None);
            ws.set_onmessage(None);
            ws.set_onclose(None);
            ws.set_onerror(None);
            let _ = ws.close();
        }
        inner.callbacks.clear();
    }

    fn set_timeout(ms: i32, f: impl FnOnce() + 'static) {
        if let Some(window) = web_sys::window() {
            let cb = Closure::once_into_js(f);
            let _ = window
                .set_timeout_with_callback_and_timeout_and_arguments_0(cb.unchecked_ref(), ms);
        }
    }

    fn open(handle: &Handle) {
        let (url, rt) = {
            let inner = handle.0.borrow();
            if inner.stopped {
                return;
            }
            let location = web_sys::window().map(|w| w.location());
            let protocol = location
                .as_ref()
                .and_then(|l| l.protocol().ok())
                .unwrap_or_default();
            let host = location
                .as_ref()
                .and_then(|l| l.host().ok())
                .unwrap_or_default();
            (ws_url(&protocol, &host), inner.rt)
        };
        rt.state.set(RealtimeState::Connecting);
        let Ok(ws) = WebSocket::new(&url) else {
            schedule_reconnect(handle);
            return;
        };

        let on_message = {
            let handle = handle.clone();
            let ws = ws.clone();
            Closure::<dyn FnMut(JsValueAlias)>::new(move |ev: JsValueAlias| {
                let Ok(ev) = ev.dyn_into::<MessageEvent>() else {
                    return;
                };
                let Some(text) = ev.data().as_string() else {
                    return;
                };
                let Ok(msg) = serde_json::from_str::<WsServerMsg>(&text) else {
                    return;
                };
                on_server_message(&handle, &ws, msg);
            })
        };
        let on_close = {
            let handle = handle.clone();
            Closure::<dyn FnMut(JsValueAlias)>::new(move |ev: JsValueAlias| {
                let code = ev.dyn_into::<CloseEvent>().map_or(0, |e| e.code());
                on_closed(&handle, code);
            })
        };
        ws.set_onmessage(Some(on_message.as_ref().unchecked_ref()));
        ws.set_onclose(Some(on_close.as_ref().unchecked_ref()));
        let mut inner = handle.0.borrow_mut();
        inner.socket = Some(ws);
        inner.callbacks = vec![on_message, on_close];
    }

    fn on_server_message(handle: &Handle, ws: &WebSocket, msg: WsServerMsg) {
        match msg {
            WsServerMsg::Hello { .. } => {
                {
                    let mut inner = handle.0.borrow_mut();
                    inner.attempt = 0;
                    // Connexion neuve : le serveur ne connait aucun abonnement, on renvoie tout.
                    inner.subscribed.clear();
                    inner.rt.state.set(RealtimeState::Online);
                }
                handle.sync();
            }
            WsServerMsg::DeviceState(dto) => {
                let rt = handle.0.borrow().rt;
                rt.devices.update(|m| {
                    m.insert(dto.device_id, dto);
                });
            }
            WsServerMsg::SystemStats(dto) => handle.0.borrow().rt.system.set(Some(dto)),
            WsServerMsg::AlertCount { firing } => handle.0.borrow().rt.alerts.set(firing),
            // Les accuses et refus ne changent rien pour l'interface (un refus laisse simplement l'etat vide).
            // Un evenement d'appareil Android (SMS, appel, accuse, presence) : les pages Android rechargent leurs donnees.
            WsServerMsg::Android { .. } => handle.0.borrow().rt.android.update(|n| *n += 1),
            // Un envoi de la messagerie a progresse : la page de detail relit ses statuts.
            WsServerMsg::MessageProgress { .. } => {
                handle.0.borrow().rt.messaging.update(|n| *n += 1);
            }
            WsServerMsg::Discovery { event } => {
                handle.0.borrow().rt.discovery.update(|v| v.apply(event));
            }
            WsServerMsg::Subscribed { .. } | WsServerMsg::Refused { .. } => {}
            WsServerMsg::Ping => {
                if let Ok(pong) = serde_json::to_string(&WsClientMsg::Pong) {
                    let _ = ws.send_with_str(&pong);
                }
            }
            WsServerMsg::Notification { level, text } => {
                let (rt, id) = {
                    let mut inner = handle.0.borrow_mut();
                    inner.next_toast += 1;
                    (inner.rt, inner.next_toast)
                };
                rt.toasts.update(|t| t.push(Toast { id, level, text }));
                set_timeout(TOAST_MS, move || rt.dismiss(id));
            }
        }
    }

    fn on_closed(handle: &Handle, code: u16) {
        let rt = {
            let mut inner = handle.0.borrow_mut();
            inner.socket = None;
            inner.callbacks.clear();
            inner.subscribed.clear();
            inner.rt
        };
        rt.state.set(RealtimeState::Offline);
        if code != SESSION_ENDED {
            schedule_reconnect(handle);
        }
    }

    fn schedule_reconnect(handle: &Handle) {
        let (delay, stopped) = {
            let mut inner = handle.0.borrow_mut();
            let base = backoff_ms(inner.attempt);
            inner.attempt += 1;
            let delay = with_jitter(base, js_sys::Math::random());
            (i32::try_from(delay).unwrap_or(30_000), inner.stopped)
        };
        if stopped {
            return;
        }
        let handle = handle.clone();
        set_timeout(delay, move || open(&handle));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reconnection_delays_double_up_to_thirty_seconds() {
        let delays: Vec<u32> = (0..8).map(backoff_ms).collect();
        assert_eq!(
            delays,
            [1_000, 2_000, 4_000, 8_000, 16_000, 30_000, 30_000, 30_000]
        );
        assert_eq!(backoff_ms(u32::MAX), 30_000, "pas de debordement");
    }

    #[test]
    fn jitter_adds_at_most_a_quarter_and_never_removes_time() {
        assert_eq!(with_jitter(1_000, 0.0), 1_000);
        assert_eq!(with_jitter(1_000, 1.0), 1_250);
        assert_eq!(with_jitter(30_000, 0.5), 33_750);
        assert_eq!(
            with_jitter(1_000, 7.0),
            1_250,
            "valeur hors borne ramenee a 1"
        );
        assert_eq!(with_jitter(1_000, -3.0), 1_000);
    }

    #[test]
    fn the_websocket_url_follows_the_page_scheme_and_host() {
        assert_eq!(
            ws_url("http:", "127.0.0.1:8000"),
            "ws://127.0.0.1:8000/ws/notifications/"
        );
        assert_eq!(
            ws_url("https:", "dominet.example"),
            "wss://dominet.example/ws/notifications/"
        );
    }

    #[test]
    fn states_have_stable_names_for_the_page() {
        let names: Vec<_> = [
            RealtimeState::Connecting,
            RealtimeState::Online,
            RealtimeState::Offline,
        ]
        .iter()
        .map(|s| s.as_str())
        .collect();
        assert_eq!(names, ["connecting", "online", "offline"]);
    }

    #[test]
    fn subscriptions_are_counted_per_requester() {
        let mut c = TopicCounts::default();
        let d = Topic::Device(7);
        assert!(c.add(d.clone()), "premier demandeur : on s'abonne");
        assert!(!c.add(d.clone()), "second demandeur : deja abonne");
        assert!(!c.remove(&d), "il en reste un");
        assert!(c.wanted().contains(&d));
        assert!(c.remove(&d), "dernier demandeur : on se desabonne");
        assert!(c.wanted().is_empty());
        assert!(!c.remove(&d), "retirer un sujet inconnu ne fait rien");
        c.add(Topic::Notifications);
        assert!(
            c.wanted().is_empty(),
            "les notifications sont toujours recues, jamais demandees"
        );
    }

    #[test]
    fn the_diff_sends_only_what_changed() {
        let set = |ids: &[i64]| {
            ids.iter()
                .map(|i| Topic::Device(*i))
                .collect::<HashSet<_>>()
        };
        let (add, remove) = subscription_diff(&set(&[1, 2, 3]), &set(&[2, 9]));
        assert_eq!(add, [Topic::Device(1), Topic::Device(3)]);
        assert_eq!(remove, [Topic::Device(9)]);
        assert_eq!(subscription_diff(&set(&[2]), &set(&[2])), (vec![], vec![]));
        // Apres une reconnexion `subscribed` est vide : tout est renvoye.
        assert_eq!(
            subscription_diff(&set(&[4, 5]), &set(&[])).0,
            [Topic::Device(4), Topic::Device(5)]
        );
    }
}
