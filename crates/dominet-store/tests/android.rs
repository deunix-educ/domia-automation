// SPDX-License-Identifier: AGPL-3.0-or-later
//! Repositories of the Android relay and of messaging (R5, migration 0010). Each test has its isolated database (`make dev-up`).

use chrono::{Duration, TimeZone, Utc};
use dominet_domain::RepoError;
use dominet_domain::android::{
    AndroidDeviceDraft, CallDirection, CallEvent, CallPlan, CallState, NewOutbox, NewSms,
    OutboxOrigin, OutboxStatus, SmsDirection, canonical_token, plan_call,
};
use dominet_domain::messaging::{Channel, DeliveryStatus, MessageStatus};
use dominet_domain::repo::{
    AndroidDeviceRepo, CallRepo, MessageRepo, SmsOutboxRepo, SmsRepo, UserRepo,
};
use dominet_domain::user::{NewUser, Role, UserId, Username};
use dominet_store::PgStore;
use sqlx::PgPool;

async fn user(store: &PgStore, name: &str) -> UserId {
    store
        .users()
        .create(&NewUser {
            username: Username::parse(name).unwrap(),
            email: String::new(),
            first_name: String::new(),
            last_name: String::new(),
            mobile: String::new(),
            role: Role::Admin,
            is_active: true,
            password_hash: "x".into(),
        })
        .await
        .unwrap()
        .id
}

fn draft(name: &str) -> AndroidDeviceDraft {
    AndroidDeviceDraft {
        name: name.into(),
        phone_number: "+33612345678".into(),
    }
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn the_token_is_given_once_and_only_its_hash_is_stored(pool: PgPool) {
    let store = PgStore::new(pool.clone());
    let owner = user(&store, "alice").await;
    let devices = store.android_devices();
    let (d, token) = devices.create(owner, &draft("Pixel")).await.unwrap();
    assert_eq!(canonical_token(&token).as_deref(), Some(token.as_str()));
    assert_eq!(
        (d.owner, d.is_online, d.phone_number.as_str()),
        (owner, false, "+33612345678")
    );

    let stored: String = sqlx::query_scalar("SELECT token_hash FROM android_device WHERE id = $1")
        .bind(d.id.0)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(stored.len(), 64);
    assert!(!stored.contains(&token) && !token.contains(&stored));

    assert_eq!(
        devices.find_by_token(&token).await.unwrap().unwrap().id,
        d.id
    );
    assert!(
        devices
            .find_by_token("0b9a0f0e-1c5e-4d1c-8f0c-7a1f3e2d4c5b")
            .await
            .unwrap()
            .is_none()
    );

    // Regeneration invalidates the old token.
    let fresh = devices.regenerate_token(d.id).await.unwrap();
    assert_ne!(fresh, token);
    assert!(devices.find_by_token(&token).await.unwrap().is_none());
    assert_eq!(
        devices.find_by_token(&fresh).await.unwrap().unwrap().id,
        d.id
    );
    assert!(matches!(
        devices
            .regenerate_token(dominet_domain::android::AndroidDeviceId(999_999))
            .await,
        Err(RepoError::NotFound)
    ));
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn presence_is_tracked_and_reset_at_startup(pool: PgPool) {
    let store = PgStore::new(pool);
    let owner = user(&store, "alice").await;
    let other = user(&store, "bob").await;
    let devices = store.android_devices();
    let (a, _) = devices.create(owner, &draft("A")).await.unwrap();
    let (b, _) = devices.create(other, &draft("B")).await.unwrap();
    let at = Utc.with_ymd_and_hms(2026, 1, 2, 10, 0, 0).unwrap();
    devices.set_online(a.id, true, at).await.unwrap();
    let a = devices.get(a.id).await.unwrap().unwrap();
    assert_eq!((a.is_online, a.last_seen), (true, Some(at)));
    assert_eq!(
        devices.list_for_owner(Some(owner)).await.unwrap().len(),
        1,
        "chacun voit les siens"
    );
    assert!(
        devices
            .most_recent_online()
            .await
            .unwrap()
            .is_some_and(|d| d.id == a.id)
    );
    devices
        .set_online(b.id, true, at + Duration::minutes(1))
        .await
        .unwrap();
    assert_eq!(
        devices.most_recent_online().await.unwrap().unwrap().id,
        b.id,
        "le plus recemment vu"
    );
    devices
        .set_online(b.id, false, at + Duration::minutes(2))
        .await
        .unwrap();
    assert_eq!(devices.set_all_offline().await.unwrap(), 1);
    assert!(devices.most_recent_online().await.unwrap().is_none());
    assert!(!devices.get(a.id).await.unwrap().unwrap().is_online);
    devices.delete(b.id).await.unwrap();
    assert!(matches!(
        devices.delete(b.id).await,
        Err(RepoError::NotFound)
    ));
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn sms_are_listed_per_owner_and_marked_read(pool: PgPool) {
    let store = PgStore::new(pool);
    let alice = user(&store, "alice").await;
    let bob = user(&store, "bob").await;
    let (da, _) = store
        .android_devices()
        .create(alice, &draft("A"))
        .await
        .unwrap();
    let (db, _) = store
        .android_devices()
        .create(bob, &draft("B"))
        .await
        .unwrap();
    let t0 = Utc.with_ymd_and_hms(2026, 1, 2, 10, 0, 0).unwrap();
    let new = |device, n: i64, dir| NewSms {
        device,
        direction: dir,
        phone_number: "+33600000000".into(),
        contact_name: String::new(),
        body: format!("sms {n}"),
        is_read: dir == SmsDirection::Outgoing,
        sent_at: t0 + Duration::minutes(n),
    };
    let sms = store.sms();
    sms.insert(&new(da.id, 1, SmsDirection::Incoming))
        .await
        .unwrap();
    sms.insert(&new(da.id, 2, SmsDirection::Incoming))
        .await
        .unwrap();
    sms.insert(&new(da.id, 3, SmsDirection::Outgoing))
        .await
        .unwrap();
    sms.insert(&new(db.id, 4, SmsDirection::Incoming))
        .await
        .unwrap();

    let mine = sms.list(Some(alice), None, 10).await.unwrap();
    assert_eq!(
        mine.iter().map(|s| s.body.as_str()).collect::<Vec<_>>(),
        ["sms 3", "sms 2", "sms 1"]
    );
    assert_eq!(
        sms.list(Some(alice), Some(db.id), 10).await.unwrap().len(),
        0,
        "pas l'appareil d'un autre"
    );
    assert_eq!(sms.unread_count(Some(alice)).await.unwrap(), 2);
    assert_eq!(sms.mark_read(da.id).await.unwrap(), 2);
    assert_eq!(sms.unread_count(Some(alice)).await.unwrap(), 0);
    assert_eq!(sms.unread_count(Some(bob)).await.unwrap(), 1);
    // `None`: an administrator's view, all the devices.
    assert_eq!(sms.list(None, None, 10).await.unwrap().len(), 4);
    assert_eq!(sms.unread_count(None).await.unwrap(), 1);
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn the_call_log_follows_the_plan(pool: PgPool) {
    let store = PgStore::new(pool);
    let owner = user(&store, "alice").await;
    let (d, _) = store
        .android_devices()
        .create(owner, &draft("A"))
        .await
        .unwrap();
    let calls = store.calls();
    let t0 = Utc.with_ymd_and_hms(2026, 1, 2, 10, 0, 0).unwrap();
    let ev = |state: &str, ts, duration| CallEvent {
        state: CallState::parse(state),
        number: "0612".into(),
        contact_name: "Alice".into(),
        timestamp: Some(ts),
        duration,
    };

    let ring = ev("ringing", t0, 0);
    let created = calls
        .apply(&plan_call(d.id, &ring, None, t0))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        (created.direction, created.ended_at),
        (CallDirection::Incoming, None)
    );
    let open = calls.find_open(d.id, "0612").await.unwrap().unwrap();
    assert_eq!(open.id, created.id);

    let end = ev("ended", t0 + Duration::minutes(5), 300);
    let closed = calls
        .apply(&plan_call(d.id, &end, Some(&open), t0))
        .await
        .unwrap()
        .unwrap();
    assert_eq!((closed.id, closed.duration), (created.id, 300));
    assert_eq!(closed.ended_at, Some(t0 + Duration::minutes(5)));
    assert!(calls.find_open(d.id, "0612").await.unwrap().is_none());

    // Missed call without an open call: entry directly ended.
    let missed = calls
        .apply(&plan_call(
            d.id,
            &ev("missed", t0 + Duration::hours(1), 0),
            None,
            t0,
        ))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(missed.direction, CallDirection::Missed);
    assert!(calls.apply(&CallPlan::Nothing).await.unwrap().is_none());
    let list = calls.list(Some(owner), None, 10).await.unwrap();
    assert_eq!(list.len(), 2);
    assert_eq!(list[0].id, missed.id, "le plus recent d'abord");
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn an_ack_only_counts_for_the_owning_device_and_only_once(pool: PgPool) {
    let store = PgStore::new(pool);
    let owner = user(&store, "alice").await;
    let (a, _) = store
        .android_devices()
        .create(owner, &draft("A"))
        .await
        .unwrap();
    let (b, _) = store
        .android_devices()
        .create(owner, &draft("B"))
        .await
        .unwrap();
    let t0 = Utc.with_ymd_and_hms(2026, 1, 2, 10, 0, 0).unwrap();
    let browser = store
        .sms()
        .insert(&NewSms {
            device: a.id,
            direction: SmsDirection::Outgoing,
            phone_number: "+33600000000".into(),
            contact_name: String::new(),
            body: "salut".into(),
            is_read: true,
            sent_at: t0,
        })
        .await
        .unwrap();
    let outbox = store.sms_outbox();
    let item = outbox
        .enqueue(&NewOutbox {
            device: a.id,
            to: "+33600000000".into(),
            body: "salut".into(),
            origin: OutboxOrigin::Browser(browser.id),
        })
        .await
        .unwrap();
    assert_eq!(item.status, OutboxStatus::Pending);
    let seen = outbox.for_sms(&[browser.id]).await.unwrap();
    assert_eq!(
        (seen.len(), seen[0].id, seen[0].status),
        (1, item.id, OutboxStatus::Pending)
    );
    assert_eq!(outbox.for_sms(&[]).await.unwrap().len(), 0);
    assert_eq!(outbox.pending_for_device(a.id).await.unwrap().len(), 1);
    assert_eq!(outbox.pending_for_device(b.id).await.unwrap().len(), 0);

    // Another device cannot acknowledge.
    assert!(
        outbox
            .ack(b.id, item.id, OutboxStatus::Sent, "", t0)
            .await
            .unwrap()
            .is_none()
    );
    let done = outbox
        .ack(a.id, item.id, OutboxStatus::Failed, "no signal", t0)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        (done.status, done.error.as_str(), done.acked_at),
        (OutboxStatus::Failed, "no signal", Some(t0))
    );
    // A second acknowledgement has no effect.
    assert!(
        outbox
            .ack(a.id, item.id, OutboxStatus::Sent, "", t0)
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(
        outbox.get(item.id).await.unwrap().unwrap().status,
        OutboxStatus::Failed
    );
    assert_eq!(outbox.pending_for_device(a.id).await.unwrap().len(), 0);
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn pending_sms_expire_after_the_deadline(pool: PgPool) {
    let store = PgStore::new(pool.clone());
    let owner = user(&store, "alice").await;
    let (a, _) = store
        .android_devices()
        .create(owner, &draft("A"))
        .await
        .unwrap();
    let sms = store
        .sms()
        .insert(&NewSms {
            device: a.id,
            direction: SmsDirection::Outgoing,
            phone_number: "+33600000000".into(),
            contact_name: String::new(),
            body: "x".into(),
            is_read: true,
            sent_at: Utc::now(),
        })
        .await
        .unwrap();
    let outbox = store.sms_outbox();
    let n = NewOutbox {
        device: a.id,
        to: "+33600000000".into(),
        body: "x".into(),
        origin: OutboxOrigin::Browser(sms.id),
    };
    let old = outbox.enqueue(&n).await.unwrap();
    let recent = outbox.enqueue(&n).await.unwrap();
    sqlx::query("UPDATE sms_outbox SET created_at = now() - interval '2 minutes' WHERE id = $1")
        .bind(old.id.0)
        .execute(&pool)
        .await
        .unwrap();
    let expired = outbox
        .expire(
            Utc::now() - Duration::seconds(60),
            "delai depasse",
            Utc::now(),
        )
        .await
        .unwrap();
    assert_eq!(expired.iter().map(|e| e.id).collect::<Vec<_>>(), [old.id]);
    assert_eq!(expired[0].status, OutboxStatus::Failed);
    assert_eq!(
        outbox.get(recent.id).await.unwrap().unwrap().status,
        OutboxStatus::Pending
    );
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn a_message_is_sent_when_nothing_is_pending(pool: PgPool) {
    use DeliveryStatus::{NotApplicable as Na, Pending as P};
    let store = PgStore::new(pool);
    let sender = user(&store, "alice").await;
    let r1 = user(&store, "bob").await;
    let r2 = user(&store, "carol").await;
    let messages = store.messages();
    let (m, ds) = messages
        .create(
            sender,
            "Sujet",
            "Corps",
            Channel::Both,
            &[(r1, P, P), (r2, P, Na)],
        )
        .await
        .unwrap();
    assert_eq!(
        (m.status, m.recipients_count, ds.len()),
        (MessageStatus::Sending, 2, 2)
    );
    assert!(
        messages
            .finish_if_complete(m.id, Utc::now())
            .await
            .unwrap()
            .is_none(),
        "envois en attente"
    );

    messages
        .set_delivery(ds[0].id, Some(DeliveryStatus::Sent), None, "")
        .await
        .unwrap();
    messages
        .set_delivery(ds[1].id, Some(DeliveryStatus::Failed), None, "smtp: refuse")
        .await
        .unwrap();
    assert!(
        messages
            .finish_if_complete(m.id, Utc::now())
            .await
            .unwrap()
            .is_none(),
        "SMS de bob en attente"
    );
    let d = messages
        .set_delivery(
            ds[0].id,
            None,
            Some(DeliveryStatus::Failed),
            "pas de telephone",
        )
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        (d.email, d.sms, d.error.as_str()),
        (
            DeliveryStatus::Sent,
            DeliveryStatus::Failed,
            "pas de telephone"
        )
    );
    let done = messages
        .finish_if_complete(m.id, Utc::now())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(done.status, MessageStatus::Sent);
    assert!(done.sent_at.is_some());
    assert!(
        messages
            .finish_if_complete(m.id, Utc::now())
            .await
            .unwrap()
            .is_none(),
        "une seule fois"
    );

    // Successive errors add up.
    let d = messages
        .set_delivery(ds[0].id, None, None, "second")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(d.error, "pas de telephone\nsecond");
    assert_eq!(messages.list(10, 0).await.unwrap().len(), 1);
    assert_eq!(messages.deliveries(m.id).await.unwrap().len(), 2);
}
