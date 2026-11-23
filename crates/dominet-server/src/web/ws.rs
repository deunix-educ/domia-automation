// SPDX-License-Identifier: AGPL-3.0-or-later
//! Notifications WebSocket: `/ws/notifications/`.
//!
//! - **Authentication** by the session (cookie): without a valid session the handshake answers 401, without opening a
//!   WebSocket.
//! - **Origin**: browsers do not apply the same-origin policy to WebSocket connections; a third-party page could
//!   open this connection with the user's cookie (cross-site WebSocket hijacking). The `Origin` header
//!   is therefore checked like for unsafe requests (`csrf`).
//! - **Subjects**: the connection always receives the notifications; the rest is subscribed on demand by
//!   `subscribe` / `unsubscribe` messages (see `dominet_proto::Topic`). A subscription returns `subscribed` then, for a state
//!   subject, the last message kept by the hub; a refused request returns `refused` (invalid or forbidden subject, or
//!   too many subscriptions: 128 per connection).
//! - **Heartbeat** (`WS_HEARTBEAT_SECS`): the server sends `ping`, the browser answers `pong`; without an answer before
//!   the next heartbeat the connection is closed. At each heartbeat the **session is revalidated** (it still
//!   exists, the account is active and its password has not changed): logout, deactivation, deletion or
//!   reset therefore also cut the real-time connection, one heartbeat later at most.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::extract::State;
use axum::extract::ws::{CloseFrame, Message, WebSocket, WebSocketUpgrade};
use axum::http::HeaderMap;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use dominet_domain::repo::UserRepo;
use dominet_domain::user::User;
use dominet_proto::{RefusalReason, Topic, WsClientMsg, WsServerMsg};
use tokio::sync::broadcast::error::RecvError;
use tokio::sync::mpsc;
use tokio::task::AbortHandle;
use tower_sessions::Session;
use tower_sessions::SessionStore;
use tower_sessions::session::Id;

use super::csrf;
use crate::AppState;
use crate::auth::extract::CurrentUser;
use crate::error::AppError;
use crate::hub::{ConnectionGuard, Hub};

/// Maximum size of a message from the browser: it only sends `pong`.
const MAX_MESSAGE_BYTES: usize = 1024;

/// Application close code: the session is no longer valid.
pub const CLOSE_SESSION_ENDED: u16 = 4401;
/// Application close code: no answer to the heartbeat.
pub const CLOSE_NO_HEARTBEAT: u16 = 4408;

pub fn routes() -> Router<AppState> {
    Router::new().route("/ws/notifications/", get(notifications))
}

async fn notifications(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    session: Session,
    headers: HeaderMap,
    ws: WebSocketUpgrade,
) -> Result<Response, AppError> {
    if !csrf::verdict(&headers, &state.config.csrf_trusted_origins) {
        tracing::warn!(user_id = user.id.0, "WebSocket refused: origin not allowed");
        return Err(AppError::Forbidden);
    }
    let Some(session_id) = session.id() else {
        return Err(AppError::Unauthorized);
    };
    let Some(guard) = state.hub.try_connect(user.id.0) else {
        return Err(AppError::TooManyRequests { retry_after_s: 5 });
    };
    Ok(ws
        .max_message_size(MAX_MESSAGE_BYTES)
        .on_upgrade(move |socket| serve(socket, state, user, session_id, guard))
        .into_response())
}

async fn send(socket: &mut WebSocket, msg: &WsServerMsg) -> bool {
    match serde_json::to_string(msg) {
        Ok(json) => socket.send(Message::Text(json.into())).await.is_ok(),
        Err(_) => false,
    }
}

async fn close(socket: &mut WebSocket, code: u16, reason: &'static str) {
    let _ = socket
        .send(Message::Close(Some(CloseFrame {
            code,
            reason: reason.into(),
        })))
        .await;
}

/// The session still exists, the account is active and its password has not changed since opening.
async fn still_valid(state: &AppState, user: &User, session_id: &Id) -> bool {
    let session_alive = matches!(state.store.sessions().load(session_id).await, Ok(Some(_)));
    session_alive
        && matches!(
            state.store.users().find_by_id(user.id).await,
            Ok(Some(u)) if u.is_active && u.password_changed_at == user.password_changed_at
        )
}

/// Maximum subscriptions of a connection (notifications included): bounds the work and memory per tab.
const MAX_SUBSCRIPTIONS: usize = 128;

/// Subjects a connection is subscribed to. Each subscription is a task that copies the subject's messages to the
/// connection's mailbox; they are all stopped when the connection closes.
struct Subscriptions {
    hub: Hub,
    inbox: mpsc::Sender<Arc<WsServerMsg>>,
    active: HashMap<Topic, AbortHandle>,
}

impl Subscriptions {
    fn new(hub: Hub, inbox: mpsc::Sender<Arc<WsServerMsg>>) -> Self {
        Self {
            hub,
            inbox,
            active: HashMap::new(),
        }
    }

    /// Subscribes and returns the subject's last kept message, to send right away. The subscription is taken **before**
    /// reading the kept state: a message published between the two is not lost (at worst it arrives twice, which is
    /// harmless for a state).
    fn add(&mut self, topic: Topic) -> Option<Arc<WsServerMsg>> {
        let mut rx = self.hub.subscribe(&topic);
        let retained = self.hub.retained(&topic);
        let inbox = self.inbox.clone();
        let label = topic.to_string();
        let task = tokio::spawn(async move {
            loop {
                match rx.recv().await {
                    Ok(msg) => {
                        if inbox.send(msg).await.is_err() {
                            break;
                        }
                    }
                    Err(RecvError::Lagged(missed)) => {
                        tracing::warn!(topic = %label, missed, "WebSocket subscriber too slow: messages lost");
                    }
                    Err(RecvError::Closed) => break,
                }
            }
        });
        self.active.insert(topic, task.abort_handle());
        retained
    }

    fn remove(&mut self, topic: &Topic) {
        if let Some(task) = self.active.remove(topic) {
            task.abort();
        }
    }
}

impl Drop for Subscriptions {
    fn drop(&mut self) {
        for task in self.active.values() {
            task.abort();
        }
    }
}

/// The device's owner or an administrator (like the reference implementation's Android page).
async fn may_follow_android(state: &AppState, user: &User, device: i64) -> bool {
    use dominet_domain::android::AndroidDeviceId;
    use dominet_domain::repo::AndroidDeviceRepo;
    matches!(
        state.store.android_devices().get(AndroidDeviceId(device)).await,
        Ok(Some(d)) if d.owner == user.id || user.role.is_admin()
    )
}

/// The camera exists and is active: any logged-in account may follow its events (like the viewing page).
async fn may_follow_camera(state: &AppState, uuid: uuid::Uuid) -> bool {
    use dominet_domain::camera::CameraUuid;
    use dominet_domain::repo::CameraRepo;
    matches!(
        state.store.cameras().get_by_uuid(CameraUuid(uuid)).await,
        Ok(Some(c)) if c.config.active
    )
}

/// The sender, a recipient or an administrator (like the detail page).
async fn may_follow_message(state: &AppState, user: &User, id: i64) -> bool {
    use dominet_domain::messaging::MessageId;
    use dominet_domain::repo::MessageRepo;
    if user.role.is_admin() {
        return true;
    }
    let messages = state.store.messages();
    let Ok(Some(m)) = messages.get(MessageId(id)).await else {
        return false;
    };
    m.sender == user.id
        || messages
            .deliveries(m.id)
            .await
            .is_ok_and(|d| d.iter().any(|d| d.recipient == user.id))
}

/// Handles a subscription request; `false` if the connection is lost.
async fn on_subscribe(
    socket: &mut WebSocket,
    subs: &mut Subscriptions,
    text: &str,
    state: &AppState,
    user: &User,
) -> bool {
    let refuse = |reason| WsServerMsg::Refused {
        topic: text.chars().take(80).collect(),
        reason,
    };
    let Ok(topic) = text.parse::<Topic>() else {
        return send(socket, &refuse(RefusalReason::Invalid)).await;
    };
    // Any logged-in account reads the notifications, the statistics and the devices' state (as in the reference implementation); the other
    // subjects have their own checks, below (Android device, message, camera, discovery).
    if let Topic::Android(id) = topic
        && !may_follow_android(state, user, id).await
    {
        // Same answer for an unknown device and for another account's: nothing is revealed.
        return send(socket, &refuse(RefusalReason::Forbidden)).await;
    }
    if let Topic::Camera(uuid) = topic
        && !may_follow_camera(state, uuid).await
    {
        // Same answer for an unknown camera and for an inactive one.
        return send(socket, &refuse(RefusalReason::Forbidden)).await;
    }
    if topic == Topic::Discovery && !user.role.is_admin() {
        return send(socket, &refuse(RefusalReason::Forbidden)).await;
    }
    if let Topic::Message(id) = topic
        && !may_follow_message(state, user, id).await
    {
        return send(socket, &refuse(RefusalReason::Forbidden)).await;
    }
    if subs.active.contains_key(&topic) {
        return send(
            socket,
            &WsServerMsg::Subscribed {
                topic: topic.to_string(),
            },
        )
        .await;
    }
    if subs.active.len() >= MAX_SUBSCRIPTIONS {
        return send(socket, &refuse(RefusalReason::Limit)).await;
    }
    let retained = subs.add(topic.clone());
    if !send(
        socket,
        &WsServerMsg::Subscribed {
            topic: topic.to_string(),
        },
    )
    .await
    {
        return false;
    }
    match retained {
        Some(msg) => send(socket, &msg).await,
        None => true,
    }
}

async fn serve(
    mut socket: WebSocket,
    state: AppState,
    user: User,
    session_id: Id,
    _guard: ConnectionGuard,
) {
    let mut beat = tokio::time::interval(Duration::from_secs(u64::from(
        state.config.ws_heartbeat_secs,
    )));
    beat.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    beat.tick().await; // the first heartbeat is immediate: we consume it
    let mut awaiting_pong = false;

    if !send(
        &mut socket,
        &WsServerMsg::Hello {
            username: user.username.clone(),
        },
    )
    .await
    {
        return;
    }
    // The notifications are always received; the rest is subscribed on demand.
    let (inbox_tx, mut inbox) = mpsc::channel(64);
    let mut subs = Subscriptions::new(state.hub.clone(), inbox_tx);
    subs.add(Topic::Notifications);
    tracing::debug!(user_id = user.id.0, "websocket opened");
    loop {
        tokio::select! {
            incoming = socket.recv() => match incoming {
                Some(Ok(Message::Text(text))) => match serde_json::from_str::<WsClientMsg>(&text) {
                    Ok(WsClientMsg::Pong) => awaiting_pong = false,
                    Ok(WsClientMsg::Subscribe { topic }) => {
                        if !on_subscribe(&mut socket, &mut subs, &topic, &state, &user).await {
                            break;
                        }
                    }
                    Ok(WsClientMsg::Unsubscribe { topic }) => {
                        // Notifications cannot be unsubscribed; a badly written subject is ignored.
                        if let Ok(topic) = topic.parse::<Topic>()
                            && topic != Topic::Notifications
                        {
                            subs.remove(&topic);
                        }
                    }
                    Err(_) => {}
                },
                Some(Ok(Message::Close(_)) | Err(_)) | None => break,
                Some(Ok(_)) => {}
            },
            published = inbox.recv() => match published {
                Some(msg) => {
                    if !send(&mut socket, &msg).await {
                        break;
                    }
                }
                None => break,
            },
            _ = beat.tick() => {
                if awaiting_pong {
                    close(&mut socket, CLOSE_NO_HEARTBEAT, "no heartbeat").await;
                    break;
                }
                if !still_valid(&state, &user, &session_id).await {
                    close(&mut socket, CLOSE_SESSION_ENDED, "session ended").await;
                    break;
                }
                if !send(&mut socket, &WsServerMsg::Ping).await {
                    break;
                }
                awaiting_pong = true;
            }
        }
    }
    tracing::debug!(user_id = user.id.0, "websocket closed");
}
