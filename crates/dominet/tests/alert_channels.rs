// SPDX-License-Identifier: AGPL-3.0-or-later
//! Alerts and automation notifications served on the real channels (R5 lot 6): the **real alert engine** raises an alert,
//! the server's notifier sends an e-mail (fake SMTP), an SMS (fake Free Mobile) and pushes the alert to a connected phone
//! (WebSocket client that speaks like the application). Nothing is really sent.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use dominet_automation::alerts::{AlertEngine, AlertEvent};
use dominet_domain::alert::{AlertRuleDraft, Severity};
use dominet_domain::android::AndroidDeviceDraft;
use dominet_domain::device::{DeviceDraft, DeviceId};
use dominet_domain::repo::{AlertRepo, AlertRuleRepo, AndroidDeviceRepo, DeviceRepo};
use dominet_notify::Security;
use dominet_notify::testing::{FakeFreeMobile, FakeSmtp, SmtpBehavior};
use dominet_proto::{NotificationLevel, Topic, WsServerMsg};
use dominet_server::alerts::AlertNotifier;
use dominet_server::auth::password::Hasher;
use dominet_server::config::{Config, EmailConfig};
use dominet_server::secrets::MasterKey;
use dominet_server::{AppState, web};
use futures_util::StreamExt;
use serde_json::{Value, json};
use sqlx::PgPool;
use tokio::net::TcpStream;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::protocol::Message as WsMessage;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};

type Engine = AlertEngine<dominet_store::PgAlertRules, dominet_store::PgAlerts, AlertNotifier>;
type Ws = WebSocketStream<MaybeTlsStream<TcpStream>>;

struct Rig {
    state: AppState,
    addr: SocketAddr,
    smtp: FakeSmtp,
    sms: FakeFreeMobile,
    device: DeviceId,
}

async fn rig(pool: PgPool) -> Rig {
    let smtp = FakeSmtp::start().await;
    let sms = FakeFreeMobile::start().await;
    let key = MasterKey::generate();
    let mut config = Config::from_lookup(|k| match k {
        "DATABASE_URL" => Some("postgres://u:p@127.0.0.1:1/none".into()),
        "COOKIE_SECURE" => Some("false".into()),
        "DOMINET_MASTER_KEY" => Some(key.clone()),
        _ => None,
    })
    .unwrap();
    config.email = Some(EmailConfig {
        host: "127.0.0.1".into(),
        port: smtp.addr().port(),
        user: None,
        password: None,
        security: Security::Plain,
        timeout_secs: 5,
    });
    config.sms_free_url = sms.url();
    let state = AppState::with_hasher(
        pool,
        Arc::new(config),
        Hasher::with_params(1024, 1, 1).unwrap(),
    );
    for (k, v) in [
        ("DEFAULT_FROM_EMAIL", "maison@example.com"),
        ("EMAIL_RECIPIENT", "alice@example.com"),
        ("SMS_FREE_USER", "12345678"),
        ("SMS_FREE_KEY", "cle-secrete"),
    ] {
        state.settings.set(k, v).await.unwrap();
    }
    let device = state
        .store
        .devices()
        .create(&DeviceDraft::new("meteo_locale"))
        .await
        .unwrap()
        .id;
    let app = web::router(state.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let serve = app.into_make_service_with_connect_info::<SocketAddr>();
    tokio::spawn(async move { axum::serve(listener, serve).await.unwrap() });
    Rig {
        state,
        addr,
        smtp,
        sms,
        device,
    }
}

impl Rig {
    /// A rule "temperature > 35" on the requested channels, and the real engine plugged into the server's notifier.
    async fn engine(
        &self,
        email: bool,
        sms: bool,
        android: bool,
    ) -> Arc<
        dominet_automation::alerts::AlertEngine<
            dominet_store::PgAlertRules,
            dominet_store::PgAlerts,
            AlertNotifier,
        >,
    > {
        let mut draft = AlertRuleDraft::new("Canicule", self.device, "temperature");
        draft.threshold_num = Some(35.0);
        draft.severity = Severity::Critical;
        draft.notify_email = email;
        draft.notify_sms = sms;
        draft.notify_android = android;
        self.state.store.alert_rules().create(&draft).await.unwrap();
        let (engine, _handle, _rx) = AlertEngine::new(
            self.state.store.alert_rules(),
            self.state.store.alerts(),
            AlertNotifier::new(&self.state),
        );
        engine.reload().await;
        engine
    }

    async fn heat(&self, engine: &Engine) {
        engine
            .process(&AlertEvent {
                device: self.device,
                name: "meteo_locale".into(),
                state: json!({"temperature": 36.2}),
            })
            .await;
    }

    async fn phone(&self) -> Ws {
        let owner = self
            .state
            .auth
            .create_user(
                "alice",
                "",
                "",
                dominet_domain::user::Role::User,
                "correct horse battery",
            )
            .await
            .unwrap()
            .id;
        let (_, token) = self
            .state
            .store
            .android_devices()
            .create(
                owner,
                &AndroidDeviceDraft {
                    name: "Pixel".into(),
                    phone_number: "+33612345678".into(),
                },
            )
            .await
            .unwrap();
        let req = format!("ws://{}/ws/android/device/{token}/", self.addr)
            .into_client_request()
            .unwrap();
        let (mut ws, _) = tokio_tungstenite::connect_async(req).await.unwrap();
        assert_eq!(next_json(&mut ws).await["type"], "auth_ok");
        ws
    }

    async fn notified(&self) -> bool {
        for _ in 0..100 {
            let all = self.state.store.alerts().list(None, 10).await.unwrap();
            if all.first().is_some_and(|a| a.notified) {
                return true;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        false
    }
}

async fn next_json(ws: &mut Ws) -> Value {
    loop {
        match tokio::time::timeout(Duration::from_secs(10), ws.next())
            .await
            .expect("delai depasse")
        {
            Some(Ok(WsMessage::Text(t))) => return serde_json::from_str(&t).unwrap(),
            Some(Ok(WsMessage::Close(_)) | Err(_)) | None => panic!("connexion fermee"),
            Some(Ok(_)) => {}
        }
    }
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn a_fired_alert_is_served_on_the_three_channels_of_its_rule(pool: PgPool) {
    let r = rig(pool).await;
    let mut phone = r.phone().await;
    let mut banner = r.state.hub.subscribe(&Topic::Notifications);
    let engine = r.engine(true, true, true).await;
    r.heat(&engine).await;
    assert!(r.notified().await, "notified passe a vrai apres l'essai");

    // E-mail: to the alerts recipient, the reference's subject, long text.
    let mail = &r.smtp.mails()[0];
    assert_eq!(
        (mail.from.as_str(), mail.to[0].as_str()),
        ("maison@example.com", "alice@example.com")
    );
    assert_eq!(
        mail.header("subject"),
        ["[DomiNet Alert Critical] Canicule"]
    );
    assert!(
        mail.body().contains("Device: meteo_locale"),
        "{}",
        mail.body()
    );
    assert!(mail.body().contains("Value: 36.2"));

    // SMS: short text, identifiers from the settings.
    let sms = &r.sms.requests()[0];
    assert_eq!(
        (sms.user.as_str(), sms.pass.as_str()),
        ("12345678", "cle-secrete")
    );
    assert_eq!(sms.msg, "[DomiNet] Canicule — meteo_locale : 36.2");

    // Android: the application receives `alert_notification` (which it ignores today).
    let push = next_json(&mut phone).await;
    assert_eq!(push["type"], "alert_notification");
    assert_eq!(push["rule_name"], "Canicule");
    assert_eq!(push["severity"], "critical");
    assert_eq!(push["message"], "[DomiNet] Canicule — meteo_locale : 36.2");

    // Interface banner, as in R4.
    let WsServerMsg::Notification { level, text } = &*banner.recv().await.unwrap() else {
        panic!("bandeau attendu")
    };
    assert_eq!(*level, NotificationLevel::Error);
    assert_eq!(text, "[DomiNet] Canicule — meteo_locale : 36.2");
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn only_the_channels_of_the_rule_are_used(pool: PgPool) {
    let r = rig(pool).await;
    let engine = r.engine(true, false, false).await;
    r.heat(&engine).await;
    assert!(r.notified().await);
    assert_eq!(r.smtp.mails().len(), 1);
    assert!(r.sms.requests().is_empty(), "pas de SMS demande");
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn a_failing_channel_does_not_stop_the_others_and_the_alert_is_still_notified(pool: PgPool) {
    let r = rig(pool).await;
    r.smtp.set_behavior(SmtpBehavior::RejectRecipients);
    let engine = r.engine(true, true, false).await;
    r.heat(&engine).await;
    assert!(
        r.notified().await,
        "notified apres l'essai, echecs ou non (comme la reference)"
    );
    assert_eq!(r.smtp.mails().len(), 0);
    assert_eq!(
        r.sms.requests().len(),
        1,
        "le SMS part malgre l'e-mail refuse"
    );
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn an_unconfigured_channel_is_skipped_without_failing_the_alert(pool: PgPool) {
    let r = rig(pool).await;
    r.state.settings.set("SMS_FREE_USER", "").await.unwrap();
    let engine = r.engine(false, true, false).await;
    r.heat(&engine).await;
    assert!(r.notified().await);
    assert!(
        r.sms.requests().is_empty(),
        "sans identifiants rien ne part"
    );
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn an_automation_notification_sends_an_email_and_an_sms_only_when_asked(pool: PgPool) {
    let r = rig(pool).await;
    let none = r
        .state
        .channels
        .notify("Porte du garage ouverte", false)
        .await;
    assert_eq!(none.len(), 0);
    assert_eq!(r.smtp.mails().len(), 1);
    assert_eq!(r.smtp.mails()[0].to[0], "alice@example.com");
    assert_eq!(
        r.smtp.mails()[0].header("subject"),
        ["[DomiNet] Notification"]
    );
    assert!(r.smtp.mails()[0].body().contains("Porte du garage ouverte"));
    assert!(
        r.sms.requests().is_empty(),
        "SMS seulement si l'automation le demande"
    );

    r.state.channels.notify("Fuite d'eau", true).await;
    assert_eq!(r.smtp.mails().len(), 2);
    assert_eq!(r.sms.requests()[0].msg, "Fuite d'eau");

    // A refused e-mail is reported, and the SMS goes out anyway.
    r.smtp.set_behavior(SmtpBehavior::RejectRecipients);
    let failed = r.state.channels.notify("Encore", true).await;
    assert_eq!(failed.len(), 1);
    assert_eq!(failed[0].0, "email");
    assert_eq!(r.sms.requests().len(), 2);
}
