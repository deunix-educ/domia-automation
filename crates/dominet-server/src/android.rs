// SPDX-License-Identifier: AGPL-3.0-or-later
//! Server-side Android relay (R5, ADR 0039): the `/ws/android/device/<token>/` endpoint of the Kotlin application (unmodified)
//! and the service that sends it orders (SMS to send, alerts).
//!
//! - **Authentication** by the token in the address (canonical form, fingerprint in the database); unknown: the WebSocket opens then
//!   closes with code **4001**. The token is never logged (see `web::loggable_path`).
//! - **One connection per device**: a new connection replaces the old one (which is closed); "offline" is only set when
//!   the **current** connection closes.
//! - **Heartbeat** (D4): the server sends a control frame every 30 s and closes after 90 s without receiving anything (`OkHttp`
//!   answers by itself); at start-up all the devices go back offline.
//! - **Browsers**: the events are published on the hub's `android:<id>` subject (D5).

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use axum::Router;
use axum::extract::ws::{CloseFrame, Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Path, State};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use chrono::{DateTime, Utc};
use dominet_domain::android::{
    AndroidDevice, AndroidDeviceId, CallEvent, CallLog, CallPlan, DeviceFrame, IncomingSms,
    NewOutbox, NewSms, OutboxId, OutboxOrigin, OutboxStatus, ServerFrame, SmsAck, SmsDirection,
    SmsMessage, SmsOutbox, canonical_token, parse_device_frame, plan_call,
};
use dominet_domain::messaging::DeliveryStatus;
use dominet_domain::repo::{AndroidDeviceRepo, CallRepo, MessageRepo, SmsOutboxRepo, SmsRepo};
use dominet_proto::{AndroidEvent, CallDto, SmsDto, Topic, WsServerMsg};
use tokio::sync::mpsc;

use crate::AppState;
use crate::hub::Hub;

/// Close: unknown token (code of the reference implementation).
pub const CLOSE_UNKNOWN_TOKEN: u16 = 4001;
/// Close: another connection of the same device took over.
pub const CLOSE_REPLACED: u16 = 4009;
/// Close: nothing received during the silence delay.
pub const CLOSE_IDLE: u16 = 4408;

/// SMS acknowledgement delay (D6): beyond it, the sending is declared failed.
pub const SMS_ACK_TIMEOUT: Duration = Duration::from_secs(60);
/// Maximum size of an application frame.
const MAX_FRAME_BYTES: usize = 64 * 1024;
/// Bounds of the fields written to the database (the table's constraints are 30 and 100 characters).
const MAX_NUMBER: usize = 30;
const MAX_CONTACT: usize = 100;
const MAX_BODY: usize = 5000;
/// Pending orders per connected device.
const OUTBOX_QUEUE: usize = 32;

#[derive(Debug)]
enum Outgoing {
    Frame(ServerFrame),
    /// Another connection takes the place.
    Replaced,
}

#[derive(Debug)]
struct Conn {
    id: u64,
    tx: mpsc::Sender<Outgoing>,
}

#[derive(Debug, Clone, Copy)]
pub struct Timing {
    pub beat: Duration,
    pub idle: Duration,
}

impl Default for Timing {
    fn default() -> Self {
        Self {
            beat: Duration::from_secs(30),
            idle: Duration::from_secs(90),
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum SendError {
    #[error("appareil hors ligne")]
    Offline,
    #[error(transparent)]
    Repo(#[from] dominet_domain::RepoError),
}

/// Connected devices and the orders to send them. Cheap to clone.
#[derive(Debug, Clone)]
pub struct AndroidRelay {
    conns: Arc<Mutex<HashMap<i64, Conn>>>,
    next: Arc<AtomicU64>,
    pub timing: Timing,
}

impl Default for AndroidRelay {
    fn default() -> Self {
        Self::new(Timing::default())
    }
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

impl AndroidRelay {
    #[must_use]
    pub fn new(timing: Timing) -> Self {
        Self {
            conns: Arc::default(),
            next: Arc::default(),
            timing,
        }
    }

    /// Registers the device's connection; the old one, if any, is invited to close.
    fn register(&self, device: AndroidDeviceId) -> (u64, mpsc::Receiver<Outgoing>) {
        let id = self.next.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = mpsc::channel(OUTBOX_QUEUE);
        if let Some(old) = lock(&self.conns).insert(device.0, Conn { id, tx }) {
            let _ = old.tx.try_send(Outgoing::Replaced);
        }
        (id, rx)
    }

    /// Removes the connection; true if it was the current one (the device is then offline).
    fn unregister(&self, device: AndroidDeviceId, conn: u64) -> bool {
        let mut conns = lock(&self.conns);
        if conns.get(&device.0).is_some_and(|c| c.id == conn) {
            conns.remove(&device.0);
            return true;
        }
        false
    }

    #[must_use]
    pub fn is_connected(&self, device: AndroidDeviceId) -> bool {
        lock(&self.conns).contains_key(&device.0)
    }

    #[must_use]
    pub fn connected(&self) -> Vec<AndroidDeviceId> {
        lock(&self.conns)
            .keys()
            .map(|id| AndroidDeviceId(*id))
            .collect()
    }

    /// Addresses a frame to the device; false if it is not connected or if its queue is full.
    fn push(&self, device: AndroidDeviceId, frame: ServerFrame) -> bool {
        lock(&self.conns)
            .get(&device.0)
            .is_some_and(|c| c.tx.try_send(Outgoing::Frame(frame)).is_ok())
    }

    /// Pushes an alert to **all** the connected devices (the current application ignores it); returns the number of devices reached.
    #[must_use]
    pub fn push_alert(
        &self,
        alert_id: i64,
        rule_name: &str,
        severity: &str,
        message: &str,
    ) -> usize {
        let ids = self.connected();
        ids.into_iter()
            .filter(|id| {
                self.push(
                    *id,
                    ServerFrame::AlertNotification {
                        alert_id,
                        rule_name: rule_name.into(),
                        severity: severity.into(),
                        message: message.into(),
                    },
                )
            })
            .count()
    }

    /// Asks a device to send an SMS: a row is added to the sending queue (its identifier is the protocol's `message_id`)
    /// then the order is transmitted. The result will arrive through the application's acknowledgement.
    ///
    /// # Errors
    /// `Offline` if the device is not connected (nothing is recorded), or a repository error.
    pub async fn send_sms(
        &self,
        state: &AppState,
        device: AndroidDeviceId,
        to: &str,
        body: &str,
        origin: OutboxOrigin,
    ) -> Result<SmsOutbox, SendError> {
        if !self.is_connected(device) {
            return Err(SendError::Offline);
        }
        let item = state
            .store
            .sms_outbox()
            .enqueue(&NewOutbox {
                device,
                to: to.to_string(),
                body: body.to_string(),
                origin,
            })
            .await?;
        if !self.push(
            device,
            ServerFrame::SendSms {
                to: to.to_string(),
                body: body.to_string(),
                message_id: item.id.0,
            },
        ) {
            // The connection has just dropped: the sending will not go out, we say so right away.
            settle(
                state,
                &item,
                OutboxStatus::Failed,
                "device unreachable",
                Utc::now(),
            )
            .await;
            return Err(SendError::Offline);
        }
        Ok(item)
    }
}

// ------------------------------------------------------------------ events towards the browsers

/// SMS as shown to the interface; `out` is the row of the sending queue of an outgoing browser SMS, if there is one.
#[must_use]
pub fn sms_dto(s: &SmsMessage, out: Option<&SmsOutbox>) -> SmsDto {
    SmsDto {
        id: s.id.0,
        device_id: s.device.0,
        outbox_id: out.map(|o| o.id.0),
        delivery: out.map_or_else(String::new, |o| o.status.as_str().to_string()),
        error: out.map(|o| o.error.clone()).unwrap_or_default(),
        direction: s.direction.as_str().into(),
        phone_number: s.phone_number.clone(),
        contact_name: s.contact_name.clone(),
        body: s.body.clone(),
        is_read: s.is_read,
        sent_at: s.sent_at.to_rfc3339(),
    }
}

#[must_use]
pub fn call_dto(c: &CallLog) -> CallDto {
    CallDto {
        id: c.id.0,
        device_id: c.device.0,
        direction: c.direction.as_str().into(),
        phone_number: c.phone_number.clone(),
        contact_name: c.contact_name.clone(),
        started_at: c.started_at.to_rfc3339(),
        ended_at: c.ended_at.map(|d| d.to_rfc3339()),
        duration: c.duration,
    }
}

/// Publishes an event for the browsers subscribed to `android:<id>`.
pub fn publish(hub: &Hub, device: AndroidDeviceId, event: AndroidEvent) {
    let msg = WsServerMsg::Android {
        device_id: device.0,
        event,
    };
    let topic = Topic::Android(device.0);
    // Presence is a state (the last one is kept), the rest are events.
    if matches!(
        msg,
        WsServerMsg::Android {
            event: AndroidEvent::DeviceStatus { .. },
            ..
        }
    ) {
        let _ = hub.publish_retained(&topic, msg);
    } else {
        let _ = hub.publish(&topic, msg);
    }
}

/// First part of `s`, without cutting a character.
fn clip(s: &str, max: usize) -> String {
    s.chars().take(max).collect()
}

// ------------------------------------------------------------------ application frames

/// Handles an application frame; the possible response is to be sent back to it.
pub async fn process_frame(
    state: &AppState,
    device: &AndroidDevice,
    frame: DeviceFrame,
    now: DateTime<Utc>,
) -> Option<ServerFrame> {
    match frame {
        DeviceFrame::SmsReceived(sms) => on_sms(state, device, sms, now).await,
        DeviceFrame::CallEvent(event) => on_call(state, device, &event, now).await,
        DeviceFrame::SmsSentAck(ack) => on_ack(state, device, &ack, now).await,
        DeviceFrame::Ping => return Some(ServerFrame::Pong),
        DeviceFrame::Other => {}
    }
    None
}

async fn on_sms(state: &AppState, device: &AndroidDevice, sms: IncomingSms, now: DateTime<Utc>) {
    let number = clip(&sms.from, MAX_NUMBER);
    if number.is_empty() {
        tracing::warn!(
            device = device.id.0,
            "SMS received without a number: ignored"
        );
        return;
    }
    let saved = state
        .store
        .sms()
        .insert(&NewSms {
            device: device.id,
            direction: SmsDirection::Incoming,
            phone_number: number,
            contact_name: clip(&sms.contact_name, MAX_CONTACT),
            body: clip(&sms.body, MAX_BODY),
            is_read: false,
            sent_at: sms.timestamp.unwrap_or(now),
        })
        .await;
    match saved {
        Ok(saved) => publish(
            &state.hub,
            device.id,
            AndroidEvent::SmsReceived {
                sms: sms_dto(&saved, None),
            },
        ),
        Err(e) => tracing::error!(device = device.id.0, error = %e, "received SMS not saved"),
    }
}

async fn on_call(state: &AppState, device: &AndroidDevice, event: &CallEvent, now: DateTime<Utc>) {
    let mut event = event.clone();
    event.number = clip(&event.number, MAX_NUMBER);
    event.contact_name = clip(&event.contact_name, MAX_CONTACT);
    if event.number.is_empty() {
        tracing::warn!(device = device.id.0, "call event without a number: ignored");
        return;
    }
    let calls = state.store.calls();
    let result = async {
        let open = calls.find_open(device.id, &event.number).await?;
        let plan = plan_call(device.id, &event, open.as_ref(), now);
        if plan == CallPlan::Nothing {
            // `answered` or unknown state: nothing is written, but the browsers see the ongoing call (like the reference).
            return Ok(open);
        }
        calls.apply(&plan).await
    }
    .await;
    match result {
        Ok(Some(call)) => publish(
            &state.hub,
            device.id,
            AndroidEvent::CallEvent {
                call: call_dto(&call),
                state: event.state.as_str().into(),
            },
        ),
        Ok(None) => {}
        Err(e) => {
            tracing::error!(device = device.id.0, error = %e, "call event not saved");
        }
    }
}

async fn on_ack(state: &AppState, device: &AndroidDevice, ack: &SmsAck, now: DateTime<Utc>) {
    let Some(id) = ack.message_id else {
        return;
    };
    let status = if ack.success {
        OutboxStatus::Sent
    } else {
        OutboxStatus::Failed
    };
    let acked = state
        .store
        .sms_outbox()
        .ack(device.id, OutboxId(id), status, &clip(&ack.error, 500), now)
        .await;
    match acked {
        Ok(Some(item)) => settled(state, &item, now).await,
        // Unknown, from another device or already acknowledged (or expired): no effect.
        Ok(None) => tracing::debug!(
            device = device.id.0,
            message_id = id,
            "acknowledgement with no target"
        ),
        Err(e) => tracing::error!(device = device.id.0, error = %e, "acknowledgement not saved"),
    }
}

/// Moves a sending-queue row to its final state (device unreachable, timeout) then draws the consequences.
pub async fn settle(
    state: &AppState,
    item: &SmsOutbox,
    status: OutboxStatus,
    error: &str,
    now: DateTime<Utc>,
) {
    match state
        .store
        .sms_outbox()
        .ack(item.device, item.id, status, error, now)
        .await
    {
        Ok(Some(done)) => settled(state, &done, now).await,
        Ok(None) => {}
        Err(e) => tracing::error!(error = %e, "send state not saved"),
    }
}

/// Consequences of a row that has just received its final state: event for the browsers and, for a messaging
/// recipient, their SMS status then the message's state.
pub async fn settled(state: &AppState, item: &SmsOutbox, now: DateTime<Utc>) {
    let success = item.status == OutboxStatus::Sent;
    publish(
        &state.hub,
        item.device,
        AndroidEvent::SmsSentAck {
            message_id: item.id.0,
            success,
            error: item.error.clone(),
        },
    );
    if let OutboxOrigin::Messaging(delivery) = item.origin {
        let messages = state.store.messages();
        let status = if success {
            DeliveryStatus::Sent
        } else {
            DeliveryStatus::Failed
        };
        let result = async {
            if let Some(d) = messages
                .set_delivery(delivery, None, Some(status), &item.error)
                .await?
            {
                let done = messages.finish_if_complete(d.message, now).await?;
                crate::messaging::publish_progress(&state.hub, d.message, done.is_some());
            }
            Ok::<_, dominet_domain::RepoError>(())
        }
        .await;
        if let Err(e) = result {
            tracing::error!(error = %e, "delivery status not saved");
        }
    }
}

/// Declares failed the SMS without an acknowledgement for more than [`SMS_ACK_TIMEOUT`]; returns their number.
pub async fn expire_stale(state: &AppState, now: DateTime<Utc>) -> usize {
    let before = now - chrono::Duration::from_std(SMS_ACK_TIMEOUT).unwrap_or_default();
    match state
        .store
        .sms_outbox()
        .expire(before, "no acknowledgement from the device", now)
        .await
    {
        Ok(items) => {
            for item in &items {
                settled(state, item, now).await;
            }
            items.len()
        }
        Err(e) => {
            tracing::error!(error = %e, "cannot expire the pending SMS");
            0
        }
    }
}

/// Background task: sweeps the SMS without an acknowledgement every 10 s.
pub async fn run_expiry(state: AppState) {
    let mut tick = tokio::time::interval(Duration::from_secs(10));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        tick.tick().await;
        expire_stale(&state, Utc::now()).await;
    }
}

// ------------------------------------------------------------------ endpoint

pub fn routes() -> Router<AppState> {
    Router::new().route("/ws/android/device/{token}/", get(device_socket))
}

async fn device_socket(
    State(state): State<AppState>,
    Path(token): Path<String>,
    ws: WebSocketUpgrade,
) -> Response {
    // Malformed token: treated as unknown, without touching the database.
    let device = match canonical_token(&token) {
        Some(t) => state
            .store
            .android_devices()
            .find_by_token(&t)
            .await
            .ok()
            .flatten(),
        None => None,
    };
    ws.max_message_size(MAX_FRAME_BYTES)
        .on_upgrade(move |socket| async move {
            if let Some(device) = device {
                serve(socket, state, device).await;
            } else {
                let mut socket = socket;
                close(&mut socket, CLOSE_UNKNOWN_TOKEN, "unknown token").await;
            }
        })
        .into_response()
}

async fn close(socket: &mut WebSocket, code: u16, reason: &'static str) {
    let _ = socket
        .send(Message::Close(Some(CloseFrame {
            code,
            reason: reason.into(),
        })))
        .await;
}

async fn send_frame(socket: &mut WebSocket, frame: &ServerFrame) -> bool {
    socket
        .send(Message::Text(frame.to_json().into()))
        .await
        .is_ok()
}

async fn serve(mut socket: WebSocket, state: AppState, device: AndroidDevice) {
    let relay = state.android.clone();
    let (conn, mut orders) = relay.register(device.id);
    let devices = state.store.android_devices();
    if let Err(e) = devices.set_online(device.id, true, Utc::now()).await {
        tracing::error!(device = device.id.0, error = %e, "presence not saved");
    }
    publish(
        &state.hub,
        device.id,
        AndroidEvent::DeviceStatus { is_online: true },
    );
    tracing::info!(device = device.id.0, name = %device.name, "Android device connected");

    if send_frame(
        &mut socket,
        &ServerFrame::AuthOk {
            device_name: device.name.clone(),
        },
    )
    .await
    {
        run(&mut socket, &state, &device, &mut orders).await;
    }

    if relay.unregister(device.id, conn) {
        if let Err(e) = devices.set_online(device.id, false, Utc::now()).await {
            tracing::error!(device = device.id.0, error = %e, "presence not saved");
        }
        publish(
            &state.hub,
            device.id,
            AndroidEvent::DeviceStatus { is_online: false },
        );
    }
    tracing::info!(device = device.id.0, "Android device disconnected");
}

async fn run(
    socket: &mut WebSocket,
    state: &AppState,
    device: &AndroidDevice,
    orders: &mut mpsc::Receiver<Outgoing>,
) {
    let Timing { beat, idle } = state.android.timing;
    let mut ticker = tokio::time::interval(beat);
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    ticker.tick().await;
    let mut last_rx = Instant::now();
    loop {
        tokio::select! {
            incoming = socket.recv() => match incoming {
                Some(Ok(Message::Text(text))) => {
                    last_rx = Instant::now();
                    if let Some(frame) = parse_device_frame(&text)
                        && let Some(reply) = process_frame(state, device, frame, Utc::now()).await
                        && !send_frame(socket, &reply).await
                    {
                        return;
                    }
                }
                Some(Ok(Message::Close(_)) | Err(_)) | None => return,
                // Control frames (OkHttp sends some) and binary ones: the line is alive.
                Some(Ok(_)) => last_rx = Instant::now(),
            },
            order = orders.recv() => match order {
                Some(Outgoing::Frame(frame)) => {
                    if !send_frame(socket, &frame).await {
                        return;
                    }
                }
                Some(Outgoing::Replaced) => {
                    close(socket, CLOSE_REPLACED, "replaced").await;
                    return;
                }
                None => return,
            },
            _ = ticker.tick() => {
                if last_rx.elapsed() >= idle {
                    close(socket, CLOSE_IDLE, "idle").await;
                    return;
                }
                if socket.send(Message::Ping(Vec::new().into())).await.is_err() {
                    return;
                }
            }
        }
    }
}
