// SPDX-License-Identifier: AGPL-3.0-or-later
//! Real time on the browser side: WebSocket connection to `/ws/notifications/`, connection state and on-screen notifications.
//!
//! The connection opens when an account is logged in and closes at logout. If it drops (network, server restart),
//! it is re-established with a growing delay (1 s, 2 s, 4 s... 30 s at most, with a little randomness so that
//! the tabs do not all come back at the same instant). A `4401` close code (session ended) stops the
//! attempts. The browser answers `pong` to the server's heartbeats.
//!
//! **Subscriptions**: a component that wants to follow a subject (a device's state) calls [`Realtime::subscribe`] and
//! [`Realtime::unsubscribe`]; the subscriptions are counted (two widgets on the same device = one subscription). The connection
//! sends the differences to the server and **resends all the subscriptions at each (re)connection** (the server keeps none of them).

use std::collections::{HashMap, HashSet};

use dominet_proto::{DeviceStateDto, NotificationLevel, SystemStatsDto, Topic};
use leptos::prelude::*;

/// Delay (ms) before attempt number `attempt` (0 for the first reconnection): 1 s doubling up to 30 s.
#[must_use]
pub fn backoff_ms(attempt: u32) -> u32 {
    1_000u32.saturating_mul(1 << attempt.min(5)).min(30_000)
}

/// Reconnection delay with up to +25% randomness (`unit` in `0.0..=1.0`) to desynchronise the tabs.
#[must_use]
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)] // bounded by `clamp`: result between 0 and 7500
pub fn with_jitter(base_ms: u32, unit: f64) -> u32 {
    base_ms + (unit.clamp(0.0, 1.0) * f64::from(base_ms) * 0.25) as u32
}

/// WebSocket address of a page (`http` -> `ws`, `https` -> `wss`): always the same origin as the page.
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

/// Subscriptions requested by the components, with their number of requesters. Notifications are not there: the
/// server always sends them.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct TopicCounts(HashMap<Topic, usize>);

impl TopicCounts {
    /// Adds a requester; true if it is the first (the subscription must then be sent).
    pub fn add(&mut self, topic: Topic) -> bool {
        let n = self.0.entry(topic).or_insert(0);
        *n += 1;
        *n == 1
    }

    /// Removes a requester; true if it was the last. Removing an unknown subject does nothing.
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

    /// Subjects to follow on the server side.
    #[must_use]
    pub fn wanted(&self) -> HashSet<Topic> {
        self.0
            .keys()
            .filter(|t| **t != Topic::Notifications)
            .cloned()
            .collect()
    }
}

/// Differences to send to the server: (subjects to request, subjects to give up).
#[must_use]
pub fn subscription_diff<S: std::hash::BuildHasher>(
    wanted: &HashSet<Topic, S>,
    subscribed: &HashSet<Topic, S>,
) -> (Vec<Topic>, Vec<Topic>) {
    let mut add: Vec<_> = wanted.difference(subscribed).cloned().collect();
    let mut remove: Vec<_> = subscribed.difference(wanted).cloned().collect();
    // Stable order (tests, logs).
    add.sort_by_key(ToString::to_string);
    remove.sort_by_key(ToString::to_string);
    (add, remove)
}

/// A notification displayed on screen.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Toast {
    pub id: u32,
    pub level: NotificationLevel,
    pub text: String,
}

/// Shared real-time state (Leptos context).
#[derive(Clone, Copy, Debug)]
pub struct Realtime {
    pub state: RwSignal<RealtimeState>,
    pub toasts: RwSignal<Vec<Toast>>,
    /// Subjects requested by the components (see [`Realtime::subscribe`]).
    pub topics: RwSignal<TopicCounts>,
    /// Last state received for each followed device.
    pub devices: RwSignal<HashMap<i64, DeviceStateDto>>,
    /// Last machine resources received (`system` subject).
    pub system: RwSignal<Option<SystemStatsDto>>,
    /// Number of ongoing alerts (`alerts` subject).
    pub alerts: RwSignal<u32>,
    /// Counter of received Android events (`android:<id>` subjects): the Android pages reload when it changes.
    pub android: RwSignal<u32>,
    /// Counter of message-sending progress events (`message:<id>` subjects): the messaging pages reload.
    pub messaging: RwSignal<u32>,
    /// State of the ONVIF discovery (`discovery` subject, administrators).
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

    /// Requests a subject (counts the requesters). No effect as long as the connection is not open: it will send it on
    /// opening.
    pub fn subscribe(&self, topic: Topic) {
        self.topics.update(|t| {
            t.add(topic);
        });
    }

    /// Gives up a subject requested by [`Realtime::subscribe`].
    pub fn unsubscribe(&self, topic: &Topic) {
        self.topics.update(|t| {
            t.remove(topic);
        });
    }

    /// Last state received for this device (reactive).
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

/// The browser's WebSocket connection (`hydrate` feature only).
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

    /// Close code sent by the server when the session is no longer valid.
    const SESSION_ENDED: u16 = 4401;
    /// Display duration of a notification.
    const TOAST_MS: i32 = 8_000;

    struct Inner {
        rt: Realtime,
        socket: Option<WebSocket>,
        /// Closures tied to the current socket (they must live as long as it does).
        callbacks: Vec<Closure<dyn FnMut(JsValueAlias)>>,
        attempt: u32,
        stopped: bool,
        next_toast: u32,
        /// Subjects the server knows for the current connection (empty at each opening and each drop).
        subscribed: HashSet<Topic>,
    }

    type JsValueAlias = wasm_bindgen::JsValue;

    /// Handle on the connection; `stop` closes it for good.
    #[derive(Clone)]
    pub struct Handle(Rc<RefCell<Inner>>);

    impl std::fmt::Debug for Handle {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str("Handle")
        }
    }

    /// Opens the connection (and maintains it until `stop`).
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

        /// Sends the server the subscriptions added or removed since the last send (nothing if the connection is not
        /// open: `Hello` will restart the synchronisation).
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

    /// Detaches the current socket's handlers then closes it (without triggering a reconnection).
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
                    // Fresh connection: the server knows no subscription, we resend everything.
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
            // Acknowledgements and refusals change nothing for the interface (a refusal simply leaves the state empty).
            // An Android device event (SMS, call, acknowledgement, presence): the Android pages reload their data.
            WsServerMsg::Android { .. } => handle.0.borrow().rt.android.update(|n| *n += 1),
            // A messaging send has progressed: the detail page re-reads its statuses.
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
        // After a reconnection `subscribed` is empty: everything is resent.
        assert_eq!(
            subscription_diff(&set(&[4, 5]), &set(&[])).0,
            [Topic::Device(4), Topic::Device(5)]
        );
    }
}
