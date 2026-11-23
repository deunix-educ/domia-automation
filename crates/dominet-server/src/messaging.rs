// SPDX-License-Identifier: AGPL-3.0-or-later
//! Server-side internal messaging (R5 lot 5, ADR 0041): a message to several accounts, by e-mail, SMS or both.
//!
//! Sending is **asynchronous** (plan R5, D6): [`send`] records the message and its recipients then returns; a background task
//! sends the e-mails (one message per recipient who has an address), hands the SMS to the Android device online most
//! recently seen, and updates the statuses. The browsers following the `message:<id>` subject are notified. An SMS stays
//! `pending` until the application's acknowledgement (see `android::settled`) or until the 60 s expire.

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

/// A single sending at a time for the whole server: the SMTP server is not hammered and the statuses stay ordered.
static SEND_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

pub const ERR_NO_DEVICE: &str = "no Android device connected";
pub const ERR_UNREACHABLE: &str = "device unreachable";
pub const ERR_RESTART: &str = "interrupted by a server restart";

/// Recipients of a message: `all` = all the active accounts, `staff` = the active administrators (both groups
/// add up with the chosen accounts, like the reference; `all` covers `staff`). No duplicates, by username.
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

/// Warns the browsers that a sending has progressed.
pub fn publish_progress(hub: &Hub, message: dominet_domain::messaging::MessageId, complete: bool) {
    let _ = hub.publish(
        &Topic::Message(message.0),
        WsServerMsg::MessageProgress {
            message_id: message.0,
            complete,
        },
    );
}

/// Records the message (state `sending`) and starts its sending in a background task.
///
/// # Errors
/// Repository error (nothing is sent).
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

/// Sends each e-mail and hands over each SMS; the message becomes `sent` when nothing is pending any more.
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

/// Hands the SMS to the device online most recently seen (of any owner, like the reference); the acknowledgement will come later.
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

/// At start-up: a message stays `sending` if the server stopped during sending. Its pending sendings are declared
/// failed (we do not know whether they went out) and the message is closed. Returns the number of messages taken over.
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

/// For the pages: the accounts by identifier.
///
/// # Errors
/// Repository error.
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

/// The chosen channel does not depend on case.
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
        // An explicitly chosen account is served even if inactive (like the reference).
        assert_eq!(names("", &[3]), ["ann"]);
        assert_eq!(names("", &[]), Vec::<&str>::new());
        assert_eq!(names("tous", &[]), Vec::<&str>::new());
        assert_eq!(names("all", &[3, 2]), ["ann", "bob", "zoe"]);
    }
}
