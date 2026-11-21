// SPDX-License-Identifier: AGPL-3.0-or-later
//! WebSocket des notifications : `/ws/notifications/`.
//!
//! - **Authentification** par la session (cookie) : sans session valide la poignee de main repond 401, sans ouvrir de
//!   WebSocket.
//! - **Origine** : les navigateurs n'appliquent pas la politique de meme origine aux WebSocket ; une page tierce pourrait
//!   ouvrir cette connexion avec le cookie de l'utilisateur (detournement de WebSocket inter-sites). L'en-tete `Origin`
//!   est donc controle comme pour les requetes non sures (`csrf`).
//! - **Sujets** : la connexion recoit toujours les notifications ; le reste s'abonne a la demande par messages
//!   `subscribe` / `unsubscribe` (voir `dominet_proto::Topic`). Un abonnement renvoie `subscribed` puis, pour un sujet
//!   d'etat, le dernier message retenu par le hub ; une demande refusee renvoie `refused` (sujet invalide, interdit, ou
//!   trop d'abonnements : 128 par connexion).
//! - **Battement** (`WS_HEARTBEAT_SECS`) : le serveur envoie `ping`, le navigateur repond `pong` ; sans reponse avant
//!   le battement suivant la connexion est fermee. A chaque battement la **session est revalidee** (elle existe
//!   toujours, le compte est actif et son mot de passe n'a pas change) : deconnexion, desactivation, suppression ou
//!   reinitialisation coupent donc aussi la connexion temps reel, au plus un battement plus tard.

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

/// Taille maximale d'un message du navigateur : il n'envoie que `pong`.
const MAX_MESSAGE_BYTES: usize = 1024;

/// Code de fermeture applicatif : la session n'est plus valide.
pub const CLOSE_SESSION_ENDED: u16 = 4401;
/// Code de fermeture applicatif : pas de reponse au battement.
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

/// La session existe encore, le compte est actif et son mot de passe n'a pas change depuis l'ouverture.
async fn still_valid(state: &AppState, user: &User, session_id: &Id) -> bool {
    let session_alive = matches!(state.store.sessions().load(session_id).await, Ok(Some(_)));
    session_alive
        && matches!(
            state.store.users().find_by_id(user.id).await,
            Ok(Some(u)) if u.is_active && u.password_changed_at == user.password_changed_at
        )
}

/// Abonnements maximum d'une connexion (notifications comprises) : borne le travail et la memoire par onglet.
const MAX_SUBSCRIPTIONS: usize = 128;

/// Sujets auxquels une connexion est abonnee. Chaque abonnement est une tache qui recopie les messages du sujet vers la
/// boite de la connexion ; elles sont toutes arretees quand la connexion se ferme.
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

    /// S'abonne et renvoie le dernier message retenu du sujet, a envoyer tout de suite. L'abonnement est pris **avant** de
    /// lire l'etat retenu : un message publie entre les deux n'est pas perdu (au pire il arrive deux fois, ce qui est
    /// sans consequence pour un etat).
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

/// Le proprietaire de l'appareil ou un administrateur (comme la page Android de l'implementation de reference).
async fn may_follow_android(state: &AppState, user: &User, device: i64) -> bool {
    use dominet_domain::android::AndroidDeviceId;
    use dominet_domain::repo::AndroidDeviceRepo;
    matches!(
        state.store.android_devices().get(AndroidDeviceId(device)).await,
        Ok(Some(d)) if d.owner == user.id || user.role.is_admin()
    )
}

/// La camera existe et est active : tout compte connecte peut suivre ses evenements (comme la page de consultation).
async fn may_follow_camera(state: &AppState, uuid: uuid::Uuid) -> bool {
    use dominet_domain::camera::CameraUuid;
    use dominet_domain::repo::CameraRepo;
    matches!(
        state.store.cameras().get_by_uuid(CameraUuid(uuid)).await,
        Ok(Some(c)) if c.config.active
    )
}

/// L'expediteur, un destinataire ou un administrateur (comme la page de detail).
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

/// Traite une demande d'abonnement ; `false` si la connexion est perdue.
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
    // Tout compte connecte lit les notifications, les statistiques et l'etat des appareils (comme dans l'implementation de reference) ; les autres
    // sujets ont leurs propres verifications, ci-dessous (appareil Android, message, camera, decouverte).
    if let Topic::Android(id) = topic
        && !may_follow_android(state, user, id).await
    {
        // Meme reponse pour un appareil inconnu et pour celui d'un autre : rien n'est revele.
        return send(socket, &refuse(RefusalReason::Forbidden)).await;
    }
    if let Topic::Camera(uuid) = topic
        && !may_follow_camera(state, uuid).await
    {
        // Meme reponse pour une camera inconnue et pour une camera inactive.
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
    beat.tick().await; // le premier battement est immediat : on le consomme
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
    // Les notifications sont toujours recues ; le reste s'abonne a la demande.
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
                        // Les notifications ne se desabonnent pas ; un sujet mal ecrit est ignore.
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
