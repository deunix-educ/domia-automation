// SPDX-License-Identifier: AGPL-3.0-or-later
//! `check_prometheus_alerts` : les messages et les SMS que Rust envoie sont ceux du **vrai implementation de reference**,
//! passage apres passage (alertes en attente ignorees, silence d'une heure, « RESOLVED » d'une alerte resolue, severite absente).

use std::time::{Duration, Instant};

use dominet_server::prometheus_alerts::{AlertMonitor, parse_alerts};
use serde_json::Value;

const FIXTURE: &str = include_str!("../../../tests/fixtures/observability/prom-alerts.json");

#[test]
fn rust_sends_what_reference_sends_step_by_step() {
    let oracle: Value = serde_json::from_str(FIXTURE).unwrap();
    let mut monitor = AlertMonitor::new(Duration::from_secs(oracle["cooldown"].as_u64().unwrap()));
    let start = Instant::now();
    for step in oracle["steps"].as_array().unwrap() {
        let t = step["t"].as_u64().unwrap();
        let alerts = parse_alerts(&serde_json::json!({"data": {"alerts": step["alerts"]}}));
        let got: Vec<(String, bool)> = monitor
            .check(&alerts, start + Duration::from_secs(t))
            .into_iter()
            .map(|n| (n.message, n.sms))
            .collect();
        let want: Vec<(String, bool)> = step["sent"]
            .as_array()
            .unwrap()
            .iter()
            .map(|n| {
                (
                    n["message"].as_str().unwrap().to_string(),
                    n["sms"].as_bool().unwrap(),
                )
            })
            .collect();
        assert_eq!(got, want, "passage a t = {t} s");
    }
}

mod live {
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    use axum::Router;
    use axum::extract::State;
    use axum::routing::get;
    use dominet_notify::testing::{FakeFreeMobile, FakeSmtp};
    use dominet_server::AppState;
    use dominet_server::auth::password::Hasher;
    use dominet_server::config::Config;
    use dominet_server::secrets::{MasterKey, SecretBox};
    use dominet_server::settings::SettingsService;
    use dominet_store::PgStore;
    use serde_json::{Value, json};
    use sqlx::PgPool;

    type Alerts = Arc<Mutex<Value>>;

    /// Corps lisible d'un e-mail recu : le tiret long force un codage (base64 ou quoted-printable) que le faux SMTP laisse tel quel.
    fn decoded(mail: &dominet_notify::testing::ReceivedMail) -> String {
        use base64::Engine;
        let body = mail.body();
        let encoding = mail
            .header("content-transfer-encoding")
            .first()
            .map(|s| s.to_ascii_lowercase());
        match encoding.as_deref() {
            Some("base64") => {
                let compact: String = body.chars().filter(|c| !c.is_whitespace()).collect();
                base64::engine::general_purpose::STANDARD
                    .decode(compact)
                    .map(|b| String::from_utf8_lossy(&b).into_owned())
                    .unwrap_or_default()
            }
            Some("quoted-printable") => quoted_printable(body),
            _ => body.to_string(),
        }
    }

    fn quoted_printable(text: &str) -> String {
        let bytes = text.replace("=\r\n", "").into_bytes();
        let mut out = vec![];
        let mut i = 0;
        while i < bytes.len() {
            let hex = |b: u8| (b as char).to_digit(16);
            if bytes[i] == b'='
                && i + 2 < bytes.len()
                && let (Some(h), Some(l)) = (hex(bytes[i + 1]), hex(bytes[i + 2]))
            {
                out.push(u8::try_from(h * 16 + l).unwrap());
                i += 3;
            } else {
                out.push(bytes[i]);
                i += 1;
            }
        }
        String::from_utf8_lossy(&out).into_owned()
    }

    async fn wait_for(what: &str, mut ok: impl FnMut() -> bool) {
        for _ in 0..80 {
            if ok() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        panic!("delai depasse : {what}");
    }

    /// Faux Prometheus : sert la liste courante des alertes sur `/api/v1/alerts`.
    async fn fake_prometheus(alerts: Alerts) -> String {
        let app = Router::new()
            .route(
                "/api/v1/alerts",
                get(|State(a): State<Alerts>| async move {
                    axum::Json(
                        json!({"status": "success", "data": {"alerts": a.lock().unwrap().clone()}}),
                    )
                }),
            )
            .with_state(alerts);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        url
    }

    #[sqlx::test(migrator = "dominet_store::MIGRATOR")]
    async fn firing_alerts_reach_the_mailbox_and_the_phone_and_resolution_follows(pool: PgPool) {
        let smtp = FakeSmtp::start().await;
        let sms = FakeFreeMobile::start().await;
        let alerts: Alerts = Arc::new(Mutex::new(json!([
            {"state": "firing", "labels": {"alertname": "DominetDown", "severity": "critical"}, "annotations": {"summary": "Hors ligne"}},
            {"state": "firing", "labels": {"alertname": "Lent", "severity": "warning"}, "annotations": {"summary": "Lent"}},
        ])));
        let prometheus = fake_prometheus(alerts.clone()).await;
        let env = [
            ("DATABASE_URL", "postgres://u:p@localhost/db".to_string()),
            ("PROMETHEUS_URL", prometheus),
            ("PROMETHEUS_ALERTS_POLL_SECS", "1".to_string()),
            ("EMAIL_HOST", "127.0.0.1".to_string()),
            ("EMAIL_PORT", smtp.addr().port().to_string()),
            ("EMAIL_USE_TLS", "false".to_string()),
            ("SMS_FREE_URL", sms.url()),
        ];
        let mut config = Config::from_lookup(|k| {
            env.iter()
                .find(|(key, _)| *key == k)
                .map(|(_, v)| v.clone())
        })
        .unwrap();
        config.email.as_mut().unwrap().security = dominet_notify::Security::Plain;
        let key = MasterKey::parse(&MasterKey::generate()).unwrap();
        let store = PgStore::new(pool.clone());
        let settings = SettingsService::new(store.settings(), Some(SecretBox::new(&key)));
        settings
            .set("EMAIL_RECIPIENT", "alice@example.com")
            .await
            .unwrap();
        settings
            .set("DEFAULT_FROM_EMAIL", "maison@example.org")
            .await
            .unwrap();
        settings.set("SMS_FREE_USER", "12345678").await.unwrap();
        settings.set("SMS_FREE_KEY", "cle-factice").await.unwrap();
        let mut state = AppState::with_hasher(
            pool,
            Arc::new(config),
            Hasher::with_params(1024, 1, 1).unwrap(),
        );
        state.settings = settings.clone();
        state.channels = dominet_server::notify::Channels::new(&state.config, settings);
        dominet_server::prometheus_alerts::spawn(&state);

        // Deux alertes distinctes : deux e-mails (l'objet de la reference), un seul SMS (la critique, 160 caracteres au plus).
        wait_for("deux e-mails", || smtp.mails().len() >= 2).await;
        let mails = smtp.mails();
        assert!(
            mails
                .iter()
                .all(|m| m.header("subject") == ["[DomiNet] Alert"])
        );
        assert!(
            mails
                .iter()
                .any(|m| decoded(m).contains("[DomiNet — FIRING] DominetDown (CRITICAL)")),
            "{:?}",
            mails[0].body()
        );
        assert!(
            mails
                .iter()
                .any(|m| decoded(m).contains("[DomiNet — FIRING] Lent (WARNING)"))
        );
        assert_eq!(sms.requests().len(), 1);
        assert!(
            sms.requests()[0]
                .msg
                .starts_with("[DomiNet — FIRING] DominetDown")
        );

        // Les passages suivants se taisent (silence d'une heure).
        tokio::time::sleep(Duration::from_millis(2500)).await;
        assert_eq!(smtp.mails().len(), 2, "pas de doublon");

        // L'alerte critique disparait : « RESOLVED » par e-mail, sans SMS.
        *alerts.lock().unwrap() = json!([
            {"state": "firing", "labels": {"alertname": "Lent", "severity": "warning"}, "annotations": {"summary": "Lent"}},
        ]);
        wait_for("l'e-mail de resolution", || smtp.mails().len() >= 3).await;
        assert!(decoded(&smtp.mails()[2]).contains("[DomiNet — RESOLVED] DominetDown (CRITICAL)"));
        assert_eq!(sms.requests().len(), 1, "pas de SMS pour une resolution");
    }
}
