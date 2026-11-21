// SPDX-License-Identifier: AGPL-3.0-or-later
//! Canaux d'envoi contre de faux serveurs (aucun envoi reel, D1).

use std::time::Duration;

use dominet_notify::testing::{FakeFreeMobile, FakeSmtp, SmtpBehavior};
use dominet_notify::{FreeMobile, NotifyError, Security, SmtpConfig, SmtpSender};

fn cfg(fake: &FakeSmtp, security: Security, auth: bool) -> SmtpConfig {
    SmtpConfig {
        host: "127.0.0.1".into(),
        port: fake.addr().port(),
        user: auth.then(|| "robot".into()),
        password: auth.then(|| "s3cret-pass".into()),
        security,
        timeout: Duration::from_secs(5),
    }
}

#[tokio::test]
async fn a_mail_reaches_the_server_with_its_headers_and_body() {
    let fake = FakeSmtp::start().await;
    let sender = SmtpSender::new(&cfg(&fake, Security::Plain, true)).unwrap();
    sender
        .send(
            "DomiNet <dominet@example.com>",
            "dest@example.com",
            "[DomiNet Alerte Critique] Canicule",
            "Valeur : 31.5\nAppareil : salon",
        )
        .await
        .unwrap();
    let mails = fake.mails();
    assert_eq!(mails.len(), 1);
    let m = &mails[0];
    assert_eq!(
        (m.from.as_str(), m.to.as_slice()),
        (
            "dominet@example.com",
            ["dest@example.com".to_string()].as_slice()
        )
    );
    assert_eq!(m.auth, Some(("robot".into(), "s3cret-pass".into())));
    assert_eq!(m.header("subject"), ["[DomiNet Alerte Critique] Canicule"]);
    assert!(m.body().contains("Valeur : 31.5"), "{}", m.data);
}

#[tokio::test]
async fn a_subject_cannot_inject_headers() {
    let fake = FakeSmtp::start().await;
    let sender = SmtpSender::new(&cfg(&fake, Security::Plain, false)).unwrap();
    sender
        .send(
            "a@example.com",
            "b@example.com",
            "Bonjour\r\nBcc: espion@example.com",
            "corps",
        )
        .await
        .unwrap();
    let m = &fake.mails()[0];
    assert!(m.header("bcc").is_empty(), "{}", m.data);
    assert_eq!(m.header("subject").len(), 1);
}

#[tokio::test]
async fn non_ascii_text_goes_through() {
    let fake = FakeSmtp::start().await;
    let sender = SmtpSender::new(&cfg(&fake, Security::Plain, false)).unwrap();
    sender
        .send(
            "a@example.com",
            "b@example.com",
            "Déclenchée à l'été",
            "Température : 31 °C ☀",
        )
        .await
        .unwrap();
    assert_eq!(fake.mails().len(), 1);
}

#[tokio::test]
async fn bad_addresses_are_refused_before_any_connection() {
    let fake = FakeSmtp::start().await;
    let sender = SmtpSender::new(&cfg(&fake, Security::Plain, false)).unwrap();
    for to in ["", "pas une adresse", "a@@b"] {
        let e = sender
            .send("a@example.com", to, "s", "c")
            .await
            .unwrap_err();
        assert!(matches!(e, NotifyError::Address(_)), "{to}: {e}");
    }
    assert_eq!(fake.mails().len(), 0);
}

#[tokio::test]
async fn refusals_are_reported_and_secrets_stay_out_of_errors() {
    let fake = FakeSmtp::start().await;
    let sender = SmtpSender::new(&cfg(&fake, Security::Plain, true)).unwrap();
    fake.set_behavior(SmtpBehavior::RejectRecipients);
    let e = sender
        .send("a@example.com", "b@example.com", "s", "c")
        .await
        .unwrap_err();
    assert!(matches!(e, NotifyError::Smtp(_)), "{e}");
    fake.set_behavior(SmtpBehavior::RejectAuth);
    let e = sender
        .send("a@example.com", "b@example.com", "s", "c")
        .await
        .unwrap_err();
    assert!(!format!("{e} {e:?}").contains("s3cret-pass"));
    assert_eq!(fake.mails().len(), 0);
}

#[tokio::test]
async fn starttls_is_required_never_downgraded() {
    // Le faux serveur n'offre pas STARTTLS : l'envoi doit echouer et rien ne doit etre transmis en clair.
    let fake = FakeSmtp::start().await;
    let sender = SmtpSender::new(&cfg(&fake, Security::StartTls, true)).unwrap();
    assert!(
        sender
            .send("a@example.com", "b@example.com", "s", "c")
            .await
            .is_err()
    );
    assert_eq!(fake.mails().len(), 0);
}

#[tokio::test]
async fn credentials_without_tls_are_only_allowed_on_loopback() {
    let fake = FakeSmtp::start().await;
    let mut c = cfg(&fake, Security::Plain, true);
    assert!(SmtpSender::new(&c).is_ok());
    c.host = "smtp.example.com".into();
    assert!(matches!(SmtpSender::new(&c), Err(NotifyError::Config(_))));
    c.host = " ".into();
    assert!(matches!(
        SmtpSender::new(&c),
        Err(NotifyError::NotConfigured(_))
    ));
    let dbg = format!("{c:?}");
    assert!(!dbg.contains("s3cret-pass") && dbg.contains("***"));
}

#[tokio::test]
async fn an_unreachable_server_fails_without_hanging() {
    let fake = FakeSmtp::start().await;
    let mut c = cfg(&fake, Security::Plain, false);
    c.port = 1; // rien n'ecoute
    c.timeout = Duration::from_secs(2);
    let sender = SmtpSender::new(&c).unwrap();
    let r = tokio::time::timeout(
        Duration::from_secs(10),
        sender.send("a@example.com", "b@example.com", "s", "c"),
    )
    .await;
    assert!(matches!(r, Ok(Err(NotifyError::Smtp(_)))), "{r:?}");
}

#[tokio::test]
async fn free_mobile_gets_user_key_and_a_message_cut_at_160_characters() {
    let fake = FakeFreeMobile::start().await;
    let sms = FreeMobile::new(&fake.url(), "12345678", "cle-secrete").unwrap();
    sms.send("[DomiNet] Canicule - salon : 31.5").await.unwrap();
    let long = "é".repeat(200);
    sms.send(&long).await.unwrap();
    let reqs = fake.requests();
    assert_eq!(
        (reqs[0].user.as_str(), reqs[0].pass.as_str()),
        ("12345678", "cle-secrete")
    );
    assert_eq!(reqs[0].msg, "[DomiNet] Canicule - salon : 31.5");
    assert_eq!(
        reqs[1].msg.chars().count(),
        160,
        "coupe en caracteres, pas en octets"
    );
}

#[tokio::test]
async fn free_mobile_http_codes_are_checked_and_the_key_never_leaks() {
    let fake = FakeFreeMobile::start().await;
    let sms = FreeMobile::new(&fake.url(), "12345678", "cle-secrete").unwrap();
    for (code, word) in [
        (400, "missing"),
        (402, "too many"),
        (403, "refused"),
        (500, "server"),
        (418, "unexpected"),
    ] {
        fake.set_status(code);
        let e = sms.send("x").await.unwrap_err().to_string();
        assert!(e.contains(word), "{code}: {e}");
        assert!(!e.contains("cle-secrete"));
    }
    // Redirection : refusee, la cle ne suit pas.
    fake.set_status(302);
    assert!(sms.send("x").await.is_err());
    // Serveur injoignable : l'erreur ne contient pas l'adresse (donc pas la cle).
    let dead = FreeMobile::new("http://127.0.0.1:1/sendmsg", "12345678", "cle-secrete").unwrap();
    let e = dead.send("x").await.unwrap_err().to_string();
    assert!(!e.contains("cle-secrete") && !e.contains("pass="), "{e}");
    let dbg = format!("{sms:?}");
    assert!(!dbg.contains("cle-secrete"));
    for (u, k) in [("", "k"), ("u", ""), ("", "")] {
        assert!(matches!(
            FreeMobile::new("http://x", u, k),
            Err(NotifyError::NotConfigured(_))
        ));
    }
}
