// SPDX-License-Identifier: AGPL-3.0-or-later
//! Messagerie interne cote serveur (R5 lot 5, ADR 0041) : un message vers plusieurs comptes, par e-mail, SMS ou les deux.
//!
//! L'envoi est **asynchrone** (plan R5, D6) : [`send`] enregistre le message et ses destinataires puis rend la main ; une tache de fond
//! envoie les e-mails (un message par destinataire qui a une adresse), confie les SMS a l'appareil Android en ligne le plus
//! recemment vu, et met les statuts a jour. Les navigateurs qui suivent le sujet `message:<id>` en sont prevenus. Un SMS reste
//! `pending` jusqu'a l'accuse de l'application (voir `android::settled`) ou jusqu'a l'expiration des 60 s.

use chrono::Utc;
use dominet_domain::RepoError;
use dominet_domain::android::OutboxOrigin;
use dominet_domain::messaging::{
    Channel, Delivery, DeliveryStatus, Message, MessageDraft, MessageStatus, initial_statuses,
};
use dominet_domain::repo::{AndroidDeviceRepo, MessageRepo, UserRepo};
use dominet_domain::user::{User, UserId};
use dominet_proto::{Topic, WsServerMsg};

use crate::AppState;
use crate::android::SendError;
use crate::hub::Hub;

/// Un seul envoi a la fois pour tout le serveur : le serveur SMTP n'est pas martele et les statuts restent ordonnes.
static SEND_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

pub const ERR_NO_DEVICE: &str = "no Android device connected";
pub const ERR_UNREACHABLE: &str = "device unreachable";
pub const ERR_RESTART: &str = "interrupted by a server restart";

/// Destinataires d'un message : `all` = tous les comptes actifs, `staff` = les administrateurs actifs (les deux groupes se
/// cumulent avec les comptes choisis, comme la reference ; `all` couvre `staff`). Sans doublon, par nom d'utilisateur.
#[must_use]
pub fn resolve_audience<'a>(users: &'a [User], audience: &str, picked: &[i64]) -> Vec<&'a User> {
    let mut out: Vec<&User> = users
        .iter()
        .filter(|u| match audience {
            "all" => u.is_active,
            "staff" => u.is_active && u.role.is_admin(),
            _ => false,
        } || picked.contains(&u.id.0))
        .collect();
    out.sort_by(|a, b| a.username.cmp(&b.username));
    out.dedup_by_key(|u| u.id);
    out
}

/// Previent les navigateurs qu'un envoi a progresse.
pub fn publish_progress(hub: &Hub, message: dominet_domain::messaging::MessageId, complete: bool) {
    let _ = hub.publish(
        &Topic::Message(message.0),
        WsServerMsg::MessageProgress {
            message_id: message.0,
            complete,
        },
    );
}

/// Enregistre le message (etat `sending`) et lance son envoi en tache de fond.
///
/// # Errors
/// Erreur de depot (rien n'est envoye).
pub async fn send(
    state: &AppState,
    sender: UserId,
    draft: &MessageDraft,
    recipients: &[&User],
) -> Result<Message, RepoError> {
    let rows: Vec<_> = recipients
        .iter()
        .map(|u| {
            let (email, sms) = initial_statuses(
                draft.channel,
                !u.email.trim().is_empty(),
                !u.mobile.trim().is_empty(),
            );
            (u.id, email, sms)
        })
        .collect();
    let (message, deliveries) = state
        .store
        .messages()
        .create(
            sender,
            draft.subject.trim(),
            &draft.body,
            draft.channel,
            &rows,
        )
        .await?;
    let numbers: std::collections::HashMap<i64, (String, String)> = recipients
        .iter()
        .map(|u| {
            (
                u.id.0,
                (u.email.trim().to_string(), u.mobile.trim().to_string()),
            )
        })
        .collect();
    tracing::info!(
        message = message.id.0,
        recipients = deliveries.len(),
        channel = draft.channel.as_str(),
        "message saved, sending in progress"
    );
    let state = state.clone();
    let queued = message.clone();
    tokio::spawn(async move {
        let _turn = SEND_LOCK.lock().await;
        dispatch(&state, &queued, &deliveries, &numbers).await;
    });
    Ok(message)
}

/// Envoie chaque e-mail et confie chaque SMS ; le message passe a `sent` quand plus rien n'est en attente.
async fn dispatch(
    state: &AppState,
    message: &Message,
    deliveries: &[Delivery],
    contacts: &std::collections::HashMap<i64, (String, String)>,
) {
    let messages = state.store.messages();
    for d in deliveries {
        let (email, mobile) = contacts.get(&d.recipient.0).cloned().unwrap_or_default();
        if d.email == DeliveryStatus::Pending {
            let result = state
                .channels
                .send_email(&email, &message.subject, &message.body)
                .await;
            let (status, error) = match result {
                Ok(()) => (DeliveryStatus::Sent, String::new()),
                Err(e) => {
                    tracing::warn!(message = message.id.0, error = %e, "e-mail not sent");
                    (DeliveryStatus::Failed, format!("email: {e}"))
                }
            };
            record(state, d, Some(status), None, &error).await;
        }
        if d.sms == DeliveryStatus::Pending
            && let Err(error) = hand_over_sms(state, message, d, &mobile).await
        {
            record(
                state,
                d,
                None,
                Some(DeliveryStatus::Failed),
                &format!("sms: {error}"),
            )
            .await;
        }
        publish_progress(&state.hub, message.id, false);
    }
    match messages.finish_if_complete(message.id, Utc::now()).await {
        Ok(done) => publish_progress(&state.hub, message.id, done.is_some()),
        Err(e) => {
            tracing::error!(message = message.id.0, error = %e, "message state not saved");
        }
    }
}

async fn record(
    state: &AppState,
    d: &Delivery,
    email: Option<DeliveryStatus>,
    sms: Option<DeliveryStatus>,
    error: &str,
) {
    if let Err(e) = state
        .store
        .messages()
        .set_delivery(d.id, email, sms, error)
        .await
    {
        tracing::error!(delivery = d.id.0, error = %e, "delivery status not saved");
    }
}

/// Confie le SMS a l'appareil en ligne le plus recemment vu (de n'importe quel proprietaire, comme la reference) ; l'accuse viendra plus tard.
async fn hand_over_sms(
    state: &AppState,
    message: &Message,
    d: &Delivery,
    mobile: &str,
) -> Result<(), String> {
    let device = state
        .store
        .android_devices()
        .most_recent_online()
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| ERR_NO_DEVICE.to_string())?;
    match state
        .android
        .send_sms(
            state,
            device.id,
            mobile,
            &message.body,
            OutboxOrigin::Messaging(d.id),
        )
        .await
    {
        Ok(_) => Ok(()),
        Err(SendError::Offline) => Err(ERR_UNREACHABLE.to_string()),
        Err(SendError::Repo(e)) => Err(e.to_string()),
    }
}

/// Au demarrage : un message reste `sending` si le serveur s'est arrete en cours d'envoi. Ses envois en attente sont declares en
/// echec (on ne sait pas s'ils sont partis) et le message est cloture. Renvoie le nombre de messages repris.
pub async fn recover_interrupted(state: &AppState) -> usize {
    let messages = state.store.messages();
    let Ok(all) = messages.list(200, 0).await else {
        tracing::warn!("interrupted messages not checked");
        return 0;
    };
    let mut n = 0;
    for m in all.iter().filter(|m| m.status == MessageStatus::Sending) {
        let Ok(deliveries) = messages.deliveries(m.id).await else {
            continue;
        };
        for d in &deliveries {
            let email = (d.email == DeliveryStatus::Pending).then_some(DeliveryStatus::Failed);
            let sms = (d.sms == DeliveryStatus::Pending).then_some(DeliveryStatus::Failed);
            if email.is_some() || sms.is_some() {
                record(state, d, email, sms, ERR_RESTART).await;
            }
        }
        let _ = messages.finish_if_complete(m.id, Utc::now()).await;
        n += 1;
    }
    n
}

/// Pour les pages : les comptes par identifiant.
///
/// # Errors
/// Erreur de depot.
pub async fn users_by_id(
    state: &AppState,
) -> Result<std::collections::HashMap<i64, User>, RepoError> {
    Ok(state
        .store
        .users()
        .list()
        .await?
        .into_iter()
        .map(|u| (u.id.0, u))
        .collect())
}

/// Le canal choisi ne depend pas de la casse.
#[must_use]
pub fn parse_channel(s: &str) -> Option<Channel> {
    Channel::parse(s.trim())
}

#[cfg(test)]
mod tests {
    use super::*;
    use dominet_domain::user::Role;

    fn user(id: i64, name: &str, role: Role, active: bool) -> User {
        User {
            id: UserId(id),
            username: name.into(),
            email: String::new(),
            first_name: String::new(),
            last_name: String::new(),
            mobile: String::new(),
            role,
            is_active: active,
            last_login_at: None,
            password_changed_at: Utc::now(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    #[test]
    fn audience_cumulates_groups_and_picks_without_duplicates() {
        let users = [
            user(1, "zoe", Role::Admin, true),
            user(2, "bob", Role::User, true),
            user(3, "ann", Role::User, false),
            user(4, "cle", Role::Admin, false),
        ];
        let names = |a: &str, p: &[i64]| {
            resolve_audience(&users, a, p)
                .iter()
                .map(|u| u.username.as_str())
                .collect::<Vec<_>>()
        };
        assert_eq!(names("all", &[]), ["bob", "zoe"]);
        assert_eq!(names("staff", &[]), ["zoe"]);
        assert_eq!(names("staff", &[2, 1]), ["bob", "zoe"]);
        // Un compte choisi explicitement est servi meme inactif (comme la reference).
        assert_eq!(names("", &[3]), ["ann"]);
        assert_eq!(names("", &[]), Vec::<&str>::new());
        assert_eq!(names("tous", &[]), Vec::<&str>::new());
        assert_eq!(names("all", &[3, 2]), ["ann", "bob", "zoe"]);
    }
}
