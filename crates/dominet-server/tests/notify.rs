// SPDX-License-Identifier: AGPL-3.0-or-later
//! Canaux d'envoi branches sur les reglages (R5 lot 2) : faux SMTP et faux Free Mobile, base isolee.

use dominet_notify::testing::{FakeFreeMobile, FakeSmtp};
use dominet_notify::{NotifyError, Security};
use dominet_server::config::{Config, EmailConfig};
use dominet_server::notify::Channels;
use dominet_server::secrets::{MasterKey, SecretBox};
use dominet_server::settings::SettingsService;
use dominet_store::PgStore;
use sqlx::PgPool;

fn config(smtp: Option<&FakeSmtp>, sms_url: &str) -> Config {
    let mut c = Config::from_lookup(|k| {
        (k == "DATABASE_URL").then(|| "postgres://u:p@localhost/db".to_string())
    })
    .unwrap();
    c.email = smtp.map(|f| EmailConfig {
        host: "127.0.0.1".into(),
        port: f.addr().port(),
        user: None,
        password: None,
        security: Security::Plain,
        timeout_secs: 5,
    });
    c.sms_free_url = sms_url.into();
    c
}

fn channels(pool: PgPool, smtp: Option<&FakeSmtp>, sms_url: &str) -> (Channels, SettingsService) {
    let settings = SettingsService::new(PgStore::new(pool).settings(), None);
    (
        Channels::new(&config(smtp, sms_url), settings.clone()),
        settings,
    )
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn alert_mail_goes_to_the_recipient_setting_with_the_sender_setting(pool: PgPool) {
    let smtp = FakeSmtp::start().await;
    let (ch, settings) = channels(pool, Some(&smtp), "http://127.0.0.1:1/");
    settings
        .set("DEFAULT_FROM_EMAIL", "maison@example.com")
        .await
        .unwrap();
    settings
        .set("EMAIL_RECIPIENT", "alice@example.com")
        .await
        .unwrap();
    ch.send_alert_email("[DomiNet Alerte Critique] Canicule", "Valeur : 31.5")
        .await
        .unwrap();
    let m = &smtp.mails()[0];
    assert_eq!(
        (m.from.as_str(), m.to[0].as_str()),
        ("maison@example.com", "alice@example.com")
    );
    assert_eq!(m.header("subject"), ["[DomiNet Alerte Critique] Canicule"]);

    // Sans destinataire, retombe sur l'expediteur comme la reference ; les reglages sont lus a chaque envoi.
    settings.set("EMAIL_RECIPIENT", "").await.unwrap();
    ch.send_alert_email("s", "c").await.unwrap();
    assert_eq!(smtp.mails()[1].to[0], "maison@example.com");
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn missing_configuration_is_reported_not_guessed(pool: PgPool) {
    let (ch, settings) = channels(pool, None, "http://127.0.0.1:1/");
    assert!(matches!(
        ch.send_email("a@example.com", "s", "c").await,
        Err(NotifyError::NotConfigured(_))
    ));
    assert!(
        matches!(
            ch.send_sms_free("x").await,
            Err(NotifyError::NotConfigured(_))
        ),
        "SMS_FREE_* vides"
    );
    settings.set("EMAIL_RECIPIENT", "").await.unwrap();
    settings.set("DEFAULT_FROM_EMAIL", "").await.unwrap();
    assert!(matches!(
        ch.send_alert_email("s", "c").await,
        Err(NotifyError::NotConfigured(_))
    ));
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn sms_uses_the_credentials_of_the_settings_and_the_configured_url(pool: PgPool) {
    let fm = FakeFreeMobile::start().await;
    let key = MasterKey::parse(&MasterKey::generate()).unwrap();
    let settings = SettingsService::new(
        PgStore::new(pool.clone()).settings(),
        Some(SecretBox::new(&key)),
    );
    let ch = Channels::new(&config(None, &fm.url()), settings.clone());
    // Sans identifiants : rien n'est envoye.
    assert!(matches!(
        ch.send_sms_free("x").await,
        Err(NotifyError::NotConfigured(_))
    ));
    settings.set("SMS_FREE_USER", "12345678").await.unwrap();
    settings.set("SMS_FREE_KEY", "cle-secrete").await.unwrap();
    ch.send_sms_free("[DomiNet] Canicule - salon : 31.5")
        .await
        .unwrap();
    let r = &fm.requests()[0];
    assert_eq!(
        (r.user.as_str(), r.pass.as_str()),
        ("12345678", "cle-secrete")
    );
    assert_eq!(r.msg, "[DomiNet] Canicule - salon : 31.5");

    // La cle est chiffree au repos, et un refus de Free Mobile ne la divulgue pas.
    let stored: String =
        sqlx::query_scalar("SELECT value FROM app_setting WHERE key = 'SMS_FREE_KEY'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(!stored.contains("cle-secrete"));
    fm.set_status(403);
    let e = ch.send_sms_free("x").await.unwrap_err().to_string();
    assert!(e.contains("403") && !e.contains("cle-secrete"), "{e}");
}
