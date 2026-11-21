// SPDX-License-Identifier: AGPL-3.0-or-later
//! Depots du relais Android et de la messagerie (R5, ADR 0037).
//!
//! Le jeton d'un appareil n'est jamais stocke : `gen_random_uuid()` le produit, SQL en calcule l'empreinte SHA-256 (hex) et le clair
//! n'est rendu qu'a la creation ou a la regeneration.

use chrono::{DateTime, Utc};
use dominet_domain::RepoError;
use dominet_domain::android::{
    AndroidDevice, AndroidDeviceDraft, AndroidDeviceId, CallDirection, CallId, CallLog, CallPlan,
    NewOutbox, NewSms, OutboxId, OutboxOrigin, OutboxStatus, SmsDirection, SmsId, SmsMessage,
    SmsOutbox,
};
use dominet_domain::messaging::{
    Channel, Delivery, DeliveryId, DeliveryStatus, Message, MessageId, MessageStatus,
};
use dominet_domain::repo::{
    AndroidDeviceRepo, CallRepo, MessageRepo, RepoResult, SmsOutboxRepo, SmsRepo,
};
use dominet_domain::user::UserId;

use crate::pg::{corrupt, repo_err};

macro_rules! pg_struct {
    ($($name:ident),*) => {$(
        #[derive(Debug, Clone)]
        pub struct $name {
            pool: sqlx::PgPool,
        }
        impl $name {
            pub(crate) fn new(pool: sqlx::PgPool) -> Self {
                Self { pool }
            }
        }
    )*};
}
pg_struct!(PgAndroidDevices, PgSms, PgCalls, PgSmsOutbox, PgMessages);

// ------------------------------------------------------------------ appareils

fn device(
    id: i64,
    owner_id: i64,
    name: String,
    phone_number: String,
    is_online: bool,
    last_seen: Option<DateTime<Utc>>,
    created_at: DateTime<Utc>,
) -> AndroidDevice {
    AndroidDevice {
        id: AndroidDeviceId(id),
        owner: UserId(owner_id),
        name,
        phone_number,
        is_online,
        last_seen,
        created_at,
    }
}

impl AndroidDeviceRepo for PgAndroidDevices {
    async fn list_for_owner(&self, owner: Option<UserId>) -> RepoResult<Vec<AndroidDevice>> {
        let rows = sqlx::query!(
            "SELECT id, owner_id, name, phone_number, is_online, last_seen, created_at
             FROM android_device WHERE ($1::bigint IS NULL OR owner_id = $1) ORDER BY created_at DESC, id DESC",
            owner.map(|o| o.0)
        )
        .fetch_all(&self.pool)
        .await
        .map_err(repo_err)?;
        Ok(rows
            .into_iter()
            .map(|r| {
                device(
                    r.id,
                    r.owner_id,
                    r.name,
                    r.phone_number,
                    r.is_online,
                    r.last_seen,
                    r.created_at,
                )
            })
            .collect())
    }

    async fn get(&self, id: AndroidDeviceId) -> RepoResult<Option<AndroidDevice>> {
        let r = sqlx::query!(
            "SELECT id, owner_id, name, phone_number, is_online, last_seen, created_at
             FROM android_device WHERE id = $1",
            id.0
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(repo_err)?;
        Ok(r.map(|r| {
            device(
                r.id,
                r.owner_id,
                r.name,
                r.phone_number,
                r.is_online,
                r.last_seen,
                r.created_at,
            )
        }))
    }

    async fn create(
        &self,
        owner: UserId,
        d: &AndroidDeviceDraft,
    ) -> RepoResult<(AndroidDevice, String)> {
        let phone = dominet_domain::user::normalize_mobile(&d.phone_number)
            .map_err(|_| corrupt("invalid number"))?;
        let r = sqlx::query!(
            r#"WITH t AS (SELECT gen_random_uuid()::text AS token)
               INSERT INTO android_device (owner_id, name, phone_number, token_hash)
               SELECT $1, $2, $3, encode(sha256(convert_to(t.token, 'UTF8')), 'hex') FROM t
               RETURNING id, owner_id, name, phone_number, is_online, last_seen, created_at,
                         (SELECT token FROM t) AS "token!""#,
            owner.0,
            d.name.trim(),
            phone
        )
        .fetch_one(&self.pool)
        .await
        .map_err(repo_err)?;
        Ok((
            device(
                r.id,
                r.owner_id,
                r.name,
                r.phone_number,
                r.is_online,
                r.last_seen,
                r.created_at,
            ),
            r.token,
        ))
    }

    async fn regenerate_token(&self, id: AndroidDeviceId) -> RepoResult<String> {
        let r = sqlx::query!(
            r#"WITH t AS (SELECT gen_random_uuid()::text AS token)
               UPDATE android_device
               SET token_hash = encode(sha256(convert_to((SELECT token FROM t), 'UTF8')), 'hex')
               WHERE id = $1
               RETURNING (SELECT token FROM t) AS "token!""#,
            id.0
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(repo_err)?;
        r.map(|r| r.token).ok_or(RepoError::NotFound)
    }

    async fn delete(&self, id: AndroidDeviceId) -> RepoResult<()> {
        let n = sqlx::query!("DELETE FROM android_device WHERE id = $1", id.0)
            .execute(&self.pool)
            .await
            .map_err(repo_err)?
            .rows_affected();
        if n == 0 {
            return Err(RepoError::NotFound);
        }
        Ok(())
    }

    async fn find_by_token(&self, token: &str) -> RepoResult<Option<AndroidDevice>> {
        let r = sqlx::query!(
            "SELECT id, owner_id, name, phone_number, is_online, last_seen, created_at
             FROM android_device WHERE token_hash = encode(sha256(convert_to($1::text, 'UTF8')), 'hex')",
            token
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(repo_err)?;
        Ok(r.map(|r| {
            device(
                r.id,
                r.owner_id,
                r.name,
                r.phone_number,
                r.is_online,
                r.last_seen,
                r.created_at,
            )
        }))
    }

    async fn set_online(
        &self,
        id: AndroidDeviceId,
        online: bool,
        at: DateTime<Utc>,
    ) -> RepoResult<()> {
        sqlx::query!(
            "UPDATE android_device SET is_online = $2, last_seen = $3 WHERE id = $1",
            id.0,
            online,
            at
        )
        .execute(&self.pool)
        .await
        .map_err(repo_err)?;
        Ok(())
    }

    async fn most_recent_online(&self) -> RepoResult<Option<AndroidDevice>> {
        let r = sqlx::query!(
            "SELECT id, owner_id, name, phone_number, is_online, last_seen, created_at
             FROM android_device WHERE is_online ORDER BY last_seen DESC NULLS LAST, id DESC LIMIT 1"
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(repo_err)?;
        Ok(r.map(|r| {
            device(
                r.id,
                r.owner_id,
                r.name,
                r.phone_number,
                r.is_online,
                r.last_seen,
                r.created_at,
            )
        }))
    }

    async fn set_all_offline(&self) -> RepoResult<u64> {
        Ok(
            sqlx::query!("UPDATE android_device SET is_online = FALSE WHERE is_online")
                .execute(&self.pool)
                .await
                .map_err(repo_err)?
                .rows_affected(),
        )
    }
}

// ------------------------------------------------------------------ SMS

#[allow(clippy::too_many_arguments)]
fn sms(
    id: i64,
    device_id: i64,
    direction: &str,
    phone_number: String,
    contact_name: String,
    body: String,
    is_read: bool,
    sent_at: DateTime<Utc>,
    received_at: DateTime<Utc>,
) -> RepoResult<SmsMessage> {
    Ok(SmsMessage {
        id: SmsId(id),
        device: AndroidDeviceId(device_id),
        direction: SmsDirection::parse(direction).ok_or_else(|| corrupt(direction))?,
        phone_number,
        contact_name,
        body,
        is_read,
        sent_at,
        received_at,
    })
}

impl SmsRepo for PgSms {
    async fn insert(&self, s: &NewSms) -> RepoResult<SmsMessage> {
        let r = sqlx::query!(
            "INSERT INTO sms_message (device_id, direction, phone_number, contact_name, body, is_read, sent_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7)
             RETURNING id, device_id, direction, phone_number, contact_name, body, is_read, sent_at, received_at",
            s.device.0,
            s.direction.as_str(),
            s.phone_number,
            s.contact_name,
            s.body,
            s.is_read,
            s.sent_at
        )
        .fetch_one(&self.pool)
        .await
        .map_err(repo_err)?;
        sms(
            r.id,
            r.device_id,
            &r.direction,
            r.phone_number,
            r.contact_name,
            r.body,
            r.is_read,
            r.sent_at,
            r.received_at,
        )
    }

    async fn list(
        &self,
        owner: Option<UserId>,
        device: Option<AndroidDeviceId>,
        limit: i64,
    ) -> RepoResult<Vec<SmsMessage>> {
        let rows = sqlx::query!(
            "SELECT m.id, m.device_id, m.direction, m.phone_number, m.contact_name, m.body, m.is_read, m.sent_at, m.received_at
             FROM sms_message m JOIN android_device d ON d.id = m.device_id
             WHERE ($1::bigint IS NULL OR d.owner_id = $1) AND ($2::bigint IS NULL OR m.device_id = $2)
             ORDER BY m.sent_at DESC, m.id DESC LIMIT $3",
            owner.map(|o| o.0),
            device.map(|d| d.0),
            limit
        )
        .fetch_all(&self.pool)
        .await
        .map_err(repo_err)?;
        rows.into_iter()
            .map(|r| {
                sms(
                    r.id,
                    r.device_id,
                    &r.direction,
                    r.phone_number,
                    r.contact_name,
                    r.body,
                    r.is_read,
                    r.sent_at,
                    r.received_at,
                )
            })
            .collect()
    }

    async fn get(&self, id: SmsId) -> RepoResult<Option<SmsMessage>> {
        let r = sqlx::query!(
            "SELECT id, device_id, direction, phone_number, contact_name, body, is_read, sent_at, received_at
             FROM sms_message WHERE id = $1",
            id.0
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(repo_err)?;
        r.map(|r| {
            sms(
                r.id,
                r.device_id,
                &r.direction,
                r.phone_number,
                r.contact_name,
                r.body,
                r.is_read,
                r.sent_at,
                r.received_at,
            )
        })
        .transpose()
    }

    async fn mark_read(&self, device: AndroidDeviceId) -> RepoResult<u64> {
        Ok(sqlx::query!(
            "UPDATE sms_message SET is_read = TRUE WHERE device_id = $1 AND direction = 'incoming' AND NOT is_read",
            device.0
        )
        .execute(&self.pool)
        .await
        .map_err(repo_err)?
        .rows_affected())
    }

    async fn unread_count(&self, owner: Option<UserId>) -> RepoResult<i64> {
        sqlx::query_scalar!(
            r#"SELECT count(*) AS "n!" FROM sms_message m JOIN android_device d ON d.id = m.device_id
               WHERE ($1::bigint IS NULL OR d.owner_id = $1) AND m.direction = 'incoming' AND NOT m.is_read"#,
            owner.map(|o| o.0)
        )
        .fetch_one(&self.pool)
        .await
        .map_err(repo_err)
    }
}

// ------------------------------------------------------------------ appels

#[allow(clippy::too_many_arguments)]
fn call(
    id: i64,
    device_id: i64,
    direction: &str,
    phone_number: String,
    contact_name: String,
    started_at: DateTime<Utc>,
    ended_at: Option<DateTime<Utc>>,
    duration: i32,
    received_at: DateTime<Utc>,
) -> RepoResult<CallLog> {
    Ok(CallLog {
        id: CallId(id),
        device: AndroidDeviceId(device_id),
        direction: CallDirection::parse(direction).ok_or_else(|| corrupt(direction))?,
        phone_number,
        contact_name,
        started_at,
        ended_at,
        duration,
        received_at,
    })
}

impl CallRepo for PgCalls {
    async fn apply(&self, plan: &CallPlan) -> RepoResult<Option<CallLog>> {
        match plan {
            CallPlan::Nothing => Ok(None),
            CallPlan::Create(c) => {
                let r = sqlx::query!(
                    "INSERT INTO call_log (device_id, direction, phone_number, contact_name, started_at, ended_at, duration)
                     VALUES ($1, $2, $3, $4, $5, $6, $7)
                     RETURNING id, device_id, direction, phone_number, contact_name, started_at, ended_at, duration, received_at",
                    c.device.0,
                    c.direction.as_str(),
                    c.phone_number,
                    c.contact_name,
                    c.started_at,
                    c.ended_at,
                    c.duration
                )
                .fetch_one(&self.pool)
                .await
                .map_err(repo_err)?;
                call(
                    r.id,
                    r.device_id,
                    &r.direction,
                    r.phone_number,
                    r.contact_name,
                    r.started_at,
                    r.ended_at,
                    r.duration,
                    r.received_at,
                )
                .map(Some)
            }
            CallPlan::Update {
                id,
                direction,
                ended_at,
                duration,
            } => {
                let r = sqlx::query!(
                    "UPDATE call_log SET direction = COALESCE($2, direction), ended_at = $3, duration = COALESCE($4, duration)
                     WHERE id = $1
                     RETURNING id, device_id, direction, phone_number, contact_name, started_at, ended_at, duration, received_at",
                    id.0,
                    direction.map(CallDirection::as_str),
                    ended_at,
                    *duration
                )
                .fetch_optional(&self.pool)
                .await
                .map_err(repo_err)?;
                r.map(|r| {
                    call(
                        r.id,
                        r.device_id,
                        &r.direction,
                        r.phone_number,
                        r.contact_name,
                        r.started_at,
                        r.ended_at,
                        r.duration,
                        r.received_at,
                    )
                })
                .transpose()
            }
        }
    }

    async fn find_open(
        &self,
        device: AndroidDeviceId,
        number: &str,
    ) -> RepoResult<Option<CallLog>> {
        let r = sqlx::query!(
            "SELECT id, device_id, direction, phone_number, contact_name, started_at, ended_at, duration, received_at
             FROM call_log WHERE device_id = $1 AND phone_number = $2 AND ended_at IS NULL
             ORDER BY started_at DESC, id DESC LIMIT 1",
            device.0,
            number
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(repo_err)?;
        r.map(|r| {
            call(
                r.id,
                r.device_id,
                &r.direction,
                r.phone_number,
                r.contact_name,
                r.started_at,
                r.ended_at,
                r.duration,
                r.received_at,
            )
        })
        .transpose()
    }

    async fn list(
        &self,
        owner: Option<UserId>,
        device: Option<AndroidDeviceId>,
        limit: i64,
    ) -> RepoResult<Vec<CallLog>> {
        let rows = sqlx::query!(
            "SELECT c.id, c.device_id, c.direction, c.phone_number, c.contact_name, c.started_at, c.ended_at, c.duration, c.received_at
             FROM call_log c JOIN android_device d ON d.id = c.device_id
             WHERE ($1::bigint IS NULL OR d.owner_id = $1) AND ($2::bigint IS NULL OR c.device_id = $2)
             ORDER BY c.started_at DESC, c.id DESC LIMIT $3",
            owner.map(|o| o.0),
            device.map(|d| d.0),
            limit
        )
        .fetch_all(&self.pool)
        .await
        .map_err(repo_err)?;
        rows.into_iter()
            .map(|r| {
                call(
                    r.id,
                    r.device_id,
                    &r.direction,
                    r.phone_number,
                    r.contact_name,
                    r.started_at,
                    r.ended_at,
                    r.duration,
                    r.received_at,
                )
            })
            .collect()
    }

    async fn get(&self, id: CallId) -> RepoResult<Option<CallLog>> {
        let r = sqlx::query!(
            "SELECT id, device_id, direction, phone_number, contact_name, started_at, ended_at, duration, received_at
             FROM call_log WHERE id = $1",
            id.0
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(repo_err)?;
        r.map(|r| {
            call(
                r.id,
                r.device_id,
                &r.direction,
                r.phone_number,
                r.contact_name,
                r.started_at,
                r.ended_at,
                r.duration,
                r.received_at,
            )
        })
        .transpose()
    }
}

// ------------------------------------------------------------------ file d'envoi

#[allow(clippy::too_many_arguments)]
fn outbox(
    id: i64,
    device_id: i64,
    to_number: String,
    body: String,
    sms_message_id: Option<i64>,
    message_recipient_id: Option<i64>,
    status: &str,
    error: String,
    created_at: DateTime<Utc>,
    acked_at: Option<DateTime<Utc>>,
) -> RepoResult<SmsOutbox> {
    let origin = match (sms_message_id, message_recipient_id) {
        (Some(s), None) => OutboxOrigin::Browser(SmsId(s)),
        (None, Some(m)) => OutboxOrigin::Messaging(DeliveryId(m)),
        _ => return Err(corrupt("origin of sms_outbox")),
    };
    Ok(SmsOutbox {
        id: OutboxId(id),
        device: AndroidDeviceId(device_id),
        to: to_number,
        body,
        origin,
        status: OutboxStatus::parse(status).ok_or_else(|| corrupt(status))?,
        error,
        created_at,
        acked_at,
    })
}

macro_rules! outbox_row {
    ($r:expr) => {{
        let r = $r;
        outbox(
            r.id,
            r.device_id,
            r.to_number,
            r.body,
            r.sms_message_id,
            r.message_recipient_id,
            &r.status,
            r.error,
            r.created_at,
            r.acked_at,
        )
    }};
}

impl SmsOutboxRepo for PgSmsOutbox {
    async fn enqueue(&self, n: &NewOutbox) -> RepoResult<SmsOutbox> {
        let (sms_id, delivery_id) = match n.origin {
            OutboxOrigin::Browser(s) => (Some(s.0), None),
            OutboxOrigin::Messaging(d) => (None, Some(d.0)),
        };
        let r = sqlx::query!(
            "INSERT INTO sms_outbox (device_id, to_number, body, sms_message_id, message_recipient_id)
             VALUES ($1, $2, $3, $4, $5)
             RETURNING id, device_id, to_number, body, sms_message_id, message_recipient_id, status, error, created_at, acked_at",
            n.device.0,
            n.to,
            n.body,
            sms_id,
            delivery_id
        )
        .fetch_one(&self.pool)
        .await
        .map_err(repo_err)?;
        outbox_row!(r)
    }

    async fn get(&self, id: OutboxId) -> RepoResult<Option<SmsOutbox>> {
        let r = sqlx::query!(
            "SELECT id, device_id, to_number, body, sms_message_id, message_recipient_id, status, error, created_at, acked_at
             FROM sms_outbox WHERE id = $1",
            id.0
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(repo_err)?;
        r.map(|r| outbox_row!(r)).transpose()
    }

    async fn for_sms(&self, ids: &[SmsId]) -> RepoResult<Vec<SmsOutbox>> {
        let raw: Vec<i64> = ids.iter().map(|i| i.0).collect();
        let rows = sqlx::query!(
            "SELECT id, device_id, to_number, body, sms_message_id, message_recipient_id, status, error, created_at, acked_at
             FROM sms_outbox WHERE sms_message_id = ANY($1) ORDER BY id",
            &raw
        )
        .fetch_all(&self.pool)
        .await
        .map_err(repo_err)?;
        rows.into_iter().map(|r| outbox_row!(r)).collect()
    }

    async fn pending_for_device(&self, device: AndroidDeviceId) -> RepoResult<Vec<SmsOutbox>> {
        let rows = sqlx::query!(
            "SELECT id, device_id, to_number, body, sms_message_id, message_recipient_id, status, error, created_at, acked_at
             FROM sms_outbox WHERE device_id = $1 AND status = 'pending' ORDER BY created_at, id",
            device.0
        )
        .fetch_all(&self.pool)
        .await
        .map_err(repo_err)?;
        rows.into_iter().map(|r| outbox_row!(r)).collect()
    }

    async fn ack(
        &self,
        device: AndroidDeviceId,
        id: OutboxId,
        status: OutboxStatus,
        error: &str,
        at: DateTime<Utc>,
    ) -> RepoResult<Option<SmsOutbox>> {
        let r = sqlx::query!(
            "UPDATE sms_outbox SET status = $3, error = $4, acked_at = $5
             WHERE id = $1 AND device_id = $2 AND status = 'pending'
             RETURNING id, device_id, to_number, body, sms_message_id, message_recipient_id, status, error, created_at, acked_at",
            id.0,
            device.0,
            status.as_str(),
            error,
            at
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(repo_err)?;
        r.map(|r| outbox_row!(r)).transpose()
    }

    async fn expire(
        &self,
        before: DateTime<Utc>,
        error: &str,
        at: DateTime<Utc>,
    ) -> RepoResult<Vec<SmsOutbox>> {
        let rows = sqlx::query!(
            "UPDATE sms_outbox SET status = 'failed', error = $2, acked_at = $3
             WHERE status = 'pending' AND created_at < $1
             RETURNING id, device_id, to_number, body, sms_message_id, message_recipient_id, status, error, created_at, acked_at",
            before,
            error,
            at
        )
        .fetch_all(&self.pool)
        .await
        .map_err(repo_err)?;
        rows.into_iter().map(|r| outbox_row!(r)).collect()
    }
}

// ------------------------------------------------------------------ messagerie

#[allow(clippy::too_many_arguments)]
fn message(
    id: i64,
    sender_id: i64,
    subject: String,
    body: String,
    channel: &str,
    status: &str,
    created_at: DateTime<Utc>,
    sent_at: Option<DateTime<Utc>>,
    recipients_count: i32,
) -> RepoResult<Message> {
    Ok(Message {
        id: MessageId(id),
        sender: UserId(sender_id),
        subject,
        body,
        channel: Channel::parse(channel).ok_or_else(|| corrupt(channel))?,
        status: MessageStatus::parse(status).ok_or_else(|| corrupt(status))?,
        created_at,
        sent_at,
        recipients_count,
    })
}

fn delivery(
    id: i64,
    message_id: i64,
    recipient_id: i64,
    email: &str,
    sms: &str,
    error: String,
    updated_at: DateTime<Utc>,
) -> RepoResult<Delivery> {
    Ok(Delivery {
        id: DeliveryId(id),
        message: MessageId(message_id),
        recipient: UserId(recipient_id),
        email: DeliveryStatus::parse(email).ok_or_else(|| corrupt(email))?,
        sms: DeliveryStatus::parse(sms).ok_or_else(|| corrupt(sms))?,
        error,
        updated_at,
    })
}

impl MessageRepo for PgMessages {
    async fn create(
        &self,
        sender: UserId,
        subject: &str,
        body: &str,
        channel: Channel,
        deliveries: &[(UserId, DeliveryStatus, DeliveryStatus)],
    ) -> RepoResult<(Message, Vec<Delivery>)> {
        let mut tx = self.pool.begin().await.map_err(repo_err)?;
        let count = i32::try_from(deliveries.len()).map_err(|_| corrupt("too many recipients"))?;
        let m = sqlx::query!(
            "INSERT INTO message (sender_id, subject, body, channel, status, recipients_count)
             VALUES ($1, $2, $3, $4, 'sending', $5)
             RETURNING id, sender_id, subject, body, channel, status, created_at, sent_at, recipients_count",
            sender.0,
            subject,
            body,
            channel.as_str(),
            count
        )
        .fetch_one(&mut *tx)
        .await
        .map_err(repo_err)?;
        let mut out = vec![];
        for (user, email, sms) in deliveries {
            let r = sqlx::query!(
                "INSERT INTO message_recipient (message_id, recipient_id, email_status, sms_status)
                 VALUES ($1, $2, $3, $4)
                 RETURNING id, message_id, recipient_id, email_status, sms_status, error, updated_at",
                m.id,
                user.0,
                email.as_str(),
                sms.as_str()
            )
            .fetch_one(&mut *tx)
            .await
            .map_err(repo_err)?;
            out.push(delivery(
                r.id,
                r.message_id,
                r.recipient_id,
                &r.email_status,
                &r.sms_status,
                r.error,
                r.updated_at,
            )?);
        }
        tx.commit().await.map_err(repo_err)?;
        let m = message(
            m.id,
            m.sender_id,
            m.subject,
            m.body,
            &m.channel,
            &m.status,
            m.created_at,
            m.sent_at,
            m.recipients_count,
        )?;
        Ok((m, out))
    }

    async fn get(&self, id: MessageId) -> RepoResult<Option<Message>> {
        let r = sqlx::query!(
            "SELECT id, sender_id, subject, body, channel, status, created_at, sent_at, recipients_count
             FROM message WHERE id = $1",
            id.0
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(repo_err)?;
        r.map(|r| {
            message(
                r.id,
                r.sender_id,
                r.subject,
                r.body,
                &r.channel,
                &r.status,
                r.created_at,
                r.sent_at,
                r.recipients_count,
            )
        })
        .transpose()
    }

    async fn list(&self, limit: i64, offset: i64) -> RepoResult<Vec<Message>> {
        let rows = sqlx::query!(
            "SELECT id, sender_id, subject, body, channel, status, created_at, sent_at, recipients_count
             FROM message ORDER BY created_at DESC, id DESC LIMIT $1 OFFSET $2",
            limit,
            offset
        )
        .fetch_all(&self.pool)
        .await
        .map_err(repo_err)?;
        rows.into_iter()
            .map(|r| {
                message(
                    r.id,
                    r.sender_id,
                    r.subject,
                    r.body,
                    &r.channel,
                    &r.status,
                    r.created_at,
                    r.sent_at,
                    r.recipients_count,
                )
            })
            .collect()
    }

    async fn deliveries(&self, id: MessageId) -> RepoResult<Vec<Delivery>> {
        let rows = sqlx::query!(
            "SELECT id, message_id, recipient_id, email_status, sms_status, error, updated_at
             FROM message_recipient WHERE message_id = $1 ORDER BY id",
            id.0
        )
        .fetch_all(&self.pool)
        .await
        .map_err(repo_err)?;
        rows.into_iter()
            .map(|r| {
                delivery(
                    r.id,
                    r.message_id,
                    r.recipient_id,
                    &r.email_status,
                    &r.sms_status,
                    r.error,
                    r.updated_at,
                )
            })
            .collect()
    }

    async fn inbox(&self, user: UserId, limit: i64) -> RepoResult<Vec<(Message, Delivery)>> {
        let rows = sqlx::query!(
            "SELECT m.id AS m_id, m.sender_id, m.subject, m.body, m.channel, m.status, m.created_at, m.sent_at, m.recipients_count,
                    r.id AS r_id, r.recipient_id, r.email_status, r.sms_status, r.error, r.updated_at
             FROM message_recipient r JOIN message m ON m.id = r.message_id
             WHERE r.recipient_id = $1 ORDER BY m.created_at DESC, m.id DESC LIMIT $2",
            user.0,
            limit
        )
        .fetch_all(&self.pool)
        .await
        .map_err(repo_err)?;
        rows.into_iter()
            .map(|r| {
                Ok((
                    message(
                        r.m_id,
                        r.sender_id,
                        r.subject,
                        r.body,
                        &r.channel,
                        &r.status,
                        r.created_at,
                        r.sent_at,
                        r.recipients_count,
                    )?,
                    delivery(
                        r.r_id,
                        r.m_id,
                        r.recipient_id,
                        &r.email_status,
                        &r.sms_status,
                        r.error,
                        r.updated_at,
                    )?,
                ))
            })
            .collect()
    }

    async fn delivery(&self, id: DeliveryId) -> RepoResult<Option<Delivery>> {
        let r = sqlx::query!(
            "SELECT id, message_id, recipient_id, email_status, sms_status, error, updated_at
             FROM message_recipient WHERE id = $1",
            id.0
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(repo_err)?;
        r.map(|r| {
            delivery(
                r.id,
                r.message_id,
                r.recipient_id,
                &r.email_status,
                &r.sms_status,
                r.error,
                r.updated_at,
            )
        })
        .transpose()
    }

    async fn set_delivery(
        &self,
        id: DeliveryId,
        email: Option<DeliveryStatus>,
        sms: Option<DeliveryStatus>,
        error: &str,
    ) -> RepoResult<Option<Delivery>> {
        let r = sqlx::query!(
            "UPDATE message_recipient
             SET email_status = COALESCE($2, email_status),
                 sms_status = COALESCE($3, sms_status),
                 error = CASE WHEN $4 = '' THEN error WHEN error = '' THEN $4 ELSE error || E'\\n' || $4 END
             WHERE id = $1
             RETURNING id, message_id, recipient_id, email_status, sms_status, error, updated_at",
            id.0,
            email.map(DeliveryStatus::as_str),
            sms.map(DeliveryStatus::as_str),
            error
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(repo_err)?;
        r.map(|r| {
            delivery(
                r.id,
                r.message_id,
                r.recipient_id,
                &r.email_status,
                &r.sms_status,
                r.error,
                r.updated_at,
            )
        })
        .transpose()
    }

    async fn finish_if_complete(
        &self,
        id: MessageId,
        at: DateTime<Utc>,
    ) -> RepoResult<Option<Message>> {
        let r = sqlx::query!(
            "UPDATE message SET status = 'sent', sent_at = $2
             WHERE id = $1 AND status <> 'sent'
               AND NOT EXISTS (SELECT 1 FROM message_recipient
                               WHERE message_id = $1 AND (email_status = 'pending' OR sms_status = 'pending'))
             RETURNING id, sender_id, subject, body, channel, status, created_at, sent_at, recipients_count",
            id.0,
            at
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(repo_err)?;
        r.map(|r| {
            message(
                r.id,
                r.sender_id,
                r.subject,
                r.body,
                &r.channel,
                &r.status,
                r.created_at,
                r.sent_at,
                r.recipients_count,
            )
        })
        .transpose()
    }

    async fn set_status(&self, id: MessageId, status: MessageStatus) -> RepoResult<()> {
        sqlx::query!(
            "UPDATE message SET status = $2 WHERE id = $1",
            id.0,
            status.as_str()
        )
        .execute(&self.pool)
        .await
        .map_err(repo_err)?;
        Ok(())
    }
}
