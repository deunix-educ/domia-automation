// SPDX-License-Identifier: AGPL-3.0-or-later
//! Relais Android (R5, ADR 0037) : appareils, SMS, journal d'appels et **protocole de l'application Kotlin** (non modifiee).
//!
//! Les trames sont celles de `CommWebSocket.kt` : l'application envoie `sms_received`, `call_event` et `sms_sent_ack`, comprend
//! `auth_ok`, `send_sms` et `pong` et ignore tout le reste. Les champs absents prennent les valeurs par defaut de l'implementation de reference
//! (`data.get(...)`), un JSON illisible est ignore.

use chrono::{DateTime, Utc};
use serde_json::{Value, json};

use crate::error::Problem;
use crate::user::{UserId, normalize_mobile};

macro_rules! id {
    ($($name:ident),*) => {$(
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
        pub struct $name(pub i64);
    )*};
}
id!(AndroidDeviceId, SmsId, CallId, OutboxId);

// ------------------------------------------------------------------ appareils

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AndroidDevice {
    pub id: AndroidDeviceId,
    pub owner: UserId,
    pub name: String,
    pub phone_number: String,
    pub is_online: bool,
    pub last_seen: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}

/// Ce que l'utilisateur saisit pour enregistrer un appareil.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AndroidDeviceDraft {
    pub name: String,
    pub phone_number: String,
}

impl AndroidDeviceDraft {
    /// Problemes champ par champ (`name`, `phone_number`).
    #[must_use]
    pub fn problems(&self) -> Vec<(&'static str, Problem)> {
        let mut out = vec![];
        if self.name.trim().is_empty() {
            out.push(("name", Problem::Required));
        } else if self.name.chars().count() > 100 {
            out.push(("name", Problem::TooLong { max: 100 }));
        }
        if self.phone_number.chars().count() > 20 {
            out.push(("phone_number", Problem::TooLong { max: 20 }));
        } else if normalize_mobile(&self.phone_number).is_err() {
            out.push(("phone_number", Problem::Invalid));
        }
        out
    }
}

/// Forme canonique d'un jeton lu dans une adresse (`/ws/android/device/<jeton>/`) : UUID en minuscules avec tirets, ou `None`.
/// Un jeton mal forme est traite comme un jeton inconnu (fermeture 4001), jamais comme une erreur.
#[must_use]
pub fn canonical_token(text: &str) -> Option<String> {
    let uuid = uuid::Uuid::parse_str(text).ok()?;
    let canonical = uuid.hyphenated().to_string();
    (canonical == text).then_some(canonical)
}

// ------------------------------------------------------------------ SMS

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SmsDirection {
    Incoming,
    Outgoing,
}

impl SmsDirection {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Incoming => "incoming",
            Self::Outgoing => "outgoing",
        }
    }

    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "incoming" => Some(Self::Incoming),
            "outgoing" => Some(Self::Outgoing),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SmsMessage {
    pub id: SmsId,
    pub device: AndroidDeviceId,
    pub direction: SmsDirection,
    pub phone_number: String,
    pub contact_name: String,
    pub body: String,
    pub is_read: bool,
    pub sent_at: DateTime<Utc>,
    pub received_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewSms {
    pub device: AndroidDeviceId,
    pub direction: SmsDirection,
    pub phone_number: String,
    pub contact_name: String,
    pub body: String,
    pub is_read: bool,
    pub sent_at: DateTime<Utc>,
}

/// Numero admis pour un envoi (`SendSmsForm.clean_to` de la reference) : `+` facultatif, puis 6 a 20 chiffres, espaces, tirets ou parentheses.
#[must_use]
pub fn is_valid_sms_number(raw: &str) -> bool {
    let t = raw.trim();
    let body = t.strip_prefix('+').unwrap_or(t);
    let n = t.chars().count();
    (6..=20).contains(&n)
        && !body.is_empty()
        && body
            .chars()
            .all(|c| c.is_ascii_digit() || matches!(c, ' ' | '-' | '(' | ')'))
}

/// Un SMS a envoyer depuis le navigateur.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SmsDraft {
    pub to: String,
    pub body: String,
}

impl SmsDraft {
    /// Problemes champ par champ (`to`, `body`).
    #[must_use]
    pub fn problems(&self) -> Vec<(&'static str, Problem)> {
        let mut out = vec![];
        if self.to.trim().is_empty() {
            out.push(("to", Problem::Required));
        } else if self.to.chars().count() > 30 {
            out.push(("to", Problem::TooLong { max: 30 }));
        } else if !is_valid_sms_number(&self.to) {
            out.push(("to", Problem::Invalid));
        }
        if self.body.trim().is_empty() {
            out.push(("body", Problem::Required));
        } else if self.body.chars().count() > 1600 {
            out.push(("body", Problem::TooLong { max: 1600 }));
        }
        out
    }
}

// ------------------------------------------------------------------ file d'envoi (accuses)

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutboxStatus {
    Pending,
    Sent,
    Failed,
}

impl OutboxStatus {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Sent => "sent",
            Self::Failed => "failed",
        }
    }

    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        [Self::Pending, Self::Sent, Self::Failed]
            .into_iter()
            .find(|x| x.as_str() == s)
    }
}

/// D'ou vient un SMS demande a un appareil.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutboxOrigin {
    /// Envoi depuis la page des SMS.
    Browser(SmsId),
    /// Destinataire d'un message de la messagerie.
    Messaging(crate::messaging::DeliveryId),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SmsOutbox {
    pub id: OutboxId,
    pub device: AndroidDeviceId,
    pub to: String,
    pub body: String,
    pub origin: OutboxOrigin,
    pub status: OutboxStatus,
    pub error: String,
    pub created_at: DateTime<Utc>,
    pub acked_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewOutbox {
    pub device: AndroidDeviceId,
    pub to: String,
    pub body: String,
    pub origin: OutboxOrigin,
}

// ------------------------------------------------------------------ appels

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CallDirection {
    Incoming,
    Outgoing,
    Missed,
}

impl CallDirection {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Incoming => "incoming",
            Self::Outgoing => "outgoing",
            Self::Missed => "missed",
        }
    }

    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        [Self::Incoming, Self::Outgoing, Self::Missed]
            .into_iter()
            .find(|x| x.as_str() == s)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CallLog {
    pub id: CallId,
    pub device: AndroidDeviceId,
    pub direction: CallDirection,
    pub phone_number: String,
    pub contact_name: String,
    pub started_at: DateTime<Utc>,
    pub ended_at: Option<DateTime<Utc>>,
    pub duration: i32,
    pub received_at: DateTime<Utc>,
}

impl CallLog {
    /// Duree `mm:ss`.
    #[must_use]
    pub fn duration_display(&self) -> String {
        format!("{:02}:{:02}", self.duration / 60, self.duration % 60)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewCall {
    pub device: AndroidDeviceId,
    pub direction: CallDirection,
    pub phone_number: String,
    pub contact_name: String,
    pub started_at: DateTime<Utc>,
    pub ended_at: Option<DateTime<Utc>>,
    pub duration: i32,
}

/// Etat d'un appel tel que l'application le raconte. Tout texte inconnu reste admis (reference l'acceptait) et n'ouvre ni ne ferme rien.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CallState {
    Ringing,
    Answered,
    Ended,
    Missed,
    Outgoing,
    Other(String),
}

impl CallState {
    #[must_use]
    pub fn parse(s: &str) -> Self {
        match s {
            "ringing" => Self::Ringing,
            "answered" => Self::Answered,
            "ended" => Self::Ended,
            "missed" => Self::Missed,
            "outgoing" => Self::Outgoing,
            other => Self::Other(other.to_string()),
        }
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        match self {
            Self::Ringing => "ringing",
            Self::Answered => "answered",
            Self::Ended => "ended",
            Self::Missed => "missed",
            Self::Outgoing => "outgoing",
            Self::Other(s) => s,
        }
    }
}

// ------------------------------------------------------------------ trames application -> serveur

/// `sms_received`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IncomingSms {
    pub from: String,
    pub contact_name: String,
    pub body: String,
    /// `None` : absent ou illisible, l'instant de reception est utilise.
    pub timestamp: Option<DateTime<Utc>>,
}

/// `call_event`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CallEvent {
    pub state: CallState,
    pub number: String,
    pub contact_name: String,
    pub timestamp: Option<DateTime<Utc>>,
    pub duration: i32,
}

/// `sms_sent_ack`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SmsAck {
    /// Identifiant renvoye tel que recu dans `send_sms` ; absent ou 0 : rien a rapprocher.
    pub message_id: Option<i64>,
    pub success: bool,
    pub error: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeviceFrame {
    SmsReceived(IncomingSms),
    CallEvent(CallEvent),
    SmsSentAck(SmsAck),
    /// `ping` JSON (l'application ne l'envoie pas ; reference y repondait `pong`).
    Ping,
    /// Type inconnu : ignore.
    Other,
}

fn text(v: &Value, key: &str) -> String {
    match v.get(key) {
        Some(Value::String(s)) => s.clone(),
        _ => String::new(),
    }
}

/// Date ISO 8601 de l'application (`ISO_OFFSET_DATE_TIME`), avec les tolerances de `parse_datetime` de reference : espace au lieu de `T`.
#[must_use]
pub fn parse_timestamp(text: &str) -> Option<DateTime<Utc>> {
    let t = text.trim();
    DateTime::parse_from_rfc3339(t)
        .or_else(|_| DateTime::parse_from_rfc3339(&t.replacen(' ', "T", 1)))
        .ok()
        .map(|d| d.with_timezone(&Utc))
}

/// Entier d'un JSON : Gson ecrit les nombres entiers tels quels, mais `75.0` est tolere (partie entiere).
#[allow(clippy::cast_possible_truncation)]
fn json_int(v: &Value) -> Option<i64> {
    v.as_i64().or_else(|| v.as_f64().map(|f| f as i64))
}

/// Lit une trame envoyee par l'application. `None` si ce n'est pas un objet JSON (ignore comme la reference).
#[must_use]
pub fn parse_device_frame(raw: &str) -> Option<DeviceFrame> {
    let v: Value = serde_json::from_str(raw).ok()?;
    let object = v.as_object()?;
    let kind = object.get("type").and_then(Value::as_str).unwrap_or("");
    Some(match kind {
        "sms_received" => DeviceFrame::SmsReceived(IncomingSms {
            from: text(&v, "from"),
            contact_name: text(&v, "contact_name"),
            body: text(&v, "body"),
            timestamp: parse_timestamp(&text(&v, "timestamp")),
        }),
        "call_event" => DeviceFrame::CallEvent(CallEvent {
            // `data.get("state", "ringing")`
            state: CallState::parse(v.get("state").and_then(Value::as_str).unwrap_or("ringing")),
            number: text(&v, "number"),
            contact_name: text(&v, "contact_name"),
            timestamp: parse_timestamp(&text(&v, "timestamp")),
            duration: v
                .get("duration")
                .and_then(json_int)
                .and_then(|d| i32::try_from(d.max(0)).ok())
                .unwrap_or(0),
        }),
        "sms_sent_ack" => DeviceFrame::SmsSentAck(SmsAck {
            message_id: v.get("message_id").and_then(json_int).filter(|m| *m != 0),
            success: v.get("success").and_then(Value::as_bool).unwrap_or(false),
            error: text(&v, "error"),
        }),
        "ping" => DeviceFrame::Ping,
        _ => DeviceFrame::Other,
    })
}

// ------------------------------------------------------------------ trames serveur -> application

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ServerFrame {
    AuthOk {
        device_name: String,
    },
    SendSms {
        to: String,
        body: String,
        message_id: i64,
    },
    Pong,
    /// Comprise par une future application ; l'actuelle l'ignore.
    AlertNotification {
        alert_id: i64,
        rule_name: String,
        severity: String,
        message: String,
    },
}

impl ServerFrame {
    /// JSON envoye a l'application (memes cles que reference).
    #[must_use]
    pub fn to_json(&self) -> String {
        match self {
            Self::AuthOk { device_name } => json!({"type": "auth_ok", "device_name": device_name}),
            Self::SendSms {
                to,
                body,
                message_id,
            } => json!({"type": "send_sms", "to": to, "body": body, "message_id": message_id}),
            Self::Pong => json!({"type": "pong"}),
            Self::AlertNotification {
                alert_id,
                rule_name,
                severity,
                message,
            } => json!({
                "type": "alert_notification", "alert_id": alert_id, "rule_name": rule_name,
                "severity": severity, "message": message
            }),
        }
        .to_string()
    }
}

// ------------------------------------------------------------------ cycle de vie d'un appel

/// Ce qu'un `call_event` change dans le journal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CallPlan {
    /// Nouvelle entree.
    Create(NewCall),
    /// L'appel ouvert est modifie.
    Update {
        id: CallId,
        direction: Option<CallDirection>,
        ended_at: DateTime<Utc>,
        duration: Option<i32>,
    },
    /// Rien n'est ecrit (etat sans effet, ou appel inconnu pour un etat qui ne cree rien).
    Nothing,
}

/// Cycle de vie du journal d'appels (`save_call_event` de la reference) : `ringing` / `outgoing` creent une entree ; `missed` / `ended`
/// ferment l'appel **ouvert** du meme numero (le plus recent : ecart voulu avec reference, qui prenait le plus ancien), ou, faute d'appel
/// ouvert, creent une entree deja terminee ; `answered` et les etats inconnus ne changent rien.
#[must_use]
pub fn plan_call(
    device: AndroidDeviceId,
    event: &CallEvent,
    open: Option<&CallLog>,
    at: DateTime<Utc>,
) -> CallPlan {
    let ts = event.timestamp.unwrap_or(at);
    match &event.state {
        CallState::Ringing | CallState::Outgoing => CallPlan::Create(NewCall {
            device,
            direction: if event.state == CallState::Outgoing {
                CallDirection::Outgoing
            } else {
                CallDirection::Incoming
            },
            phone_number: event.number.clone(),
            contact_name: event.contact_name.clone(),
            started_at: ts,
            ended_at: None,
            duration: 0,
        }),
        CallState::Missed | CallState::Ended => match open {
            Some(call) if event.state == CallState::Missed => CallPlan::Update {
                id: call.id,
                direction: Some(CallDirection::Missed),
                ended_at: ts,
                duration: None,
            },
            Some(call) => CallPlan::Update {
                id: call.id,
                direction: None,
                ended_at: ts,
                duration: Some(event.duration),
            },
            None => CallPlan::Create(NewCall {
                device,
                direction: if event.state == CallState::Missed {
                    CallDirection::Missed
                } else {
                    CallDirection::Incoming
                },
                phone_number: event.number.clone(),
                contact_name: event.contact_name.clone(),
                started_at: ts,
                ended_at: Some(ts),
                duration: event.duration,
            }),
        },
        CallState::Answered | CallState::Other(_) => CallPlan::Nothing,
    }
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;

    use super::*;

    fn at(h: u32, m: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 1, 2, h, m, 0).unwrap()
    }

    fn event(state: &str, number: &str, ts: Option<DateTime<Utc>>, duration: i32) -> CallEvent {
        CallEvent {
            state: CallState::parse(state),
            number: number.into(),
            contact_name: String::new(),
            timestamp: ts,
            duration,
        }
    }

    #[test]
    fn tokens_are_canonical_lowercase_uuids_only() {
        let t = "0b9a0f0e-1c5e-4d1c-8f0c-7a1f3e2d4c5b";
        assert_eq!(canonical_token(t).as_deref(), Some(t));
        for bad in [
            "",
            "pas-un-jeton",
            "0B9A0F0E-1C5E-4D1C-8F0C-7A1F3E2D4C5B",
            "0b9a0f0e1c5e4d1c8f0c7a1f3e2d4c5b",
            "{0b9a0f0e-1c5e-4d1c-8f0c-7a1f3e2d4c5b}",
            "0b9a0f0e-1c5e-4d1c-8f0c-7a1f3e2d4c5b/../x",
        ] {
            assert_eq!(canonical_token(bad), None, "{bad}");
        }
    }

    #[test]
    fn devices_and_sms_are_validated_field_by_field() {
        let d = |n: &str, p: &str| AndroidDeviceDraft {
            name: n.into(),
            phone_number: p.into(),
        };
        assert_eq!(d("Pixel 7", "+33 6 12 34 56 78").problems(), []);
        assert_eq!(d("Pixel", "").problems(), []);
        assert_eq!(d(" ", "").problems(), [("name", Problem::Required)]);
        assert_eq!(
            d("Pixel", "abc").problems(),
            [("phone_number", Problem::Invalid)]
        );
        assert_eq!(
            d(&"x".repeat(101), "").problems(),
            [("name", Problem::TooLong { max: 100 })]
        );
        for ok in ["+33612345678", "06 12 34 56 78", "(01) 23-45-67", "123456"] {
            assert!(is_valid_sms_number(ok), "{ok}");
        }
        for bad in [
            "",
            "12345",
            "+",
            "abc123456",
            "+33612345678901234567890",
            "06.12.34.56.78",
        ] {
            assert!(!is_valid_sms_number(bad), "{bad}");
        }
        let sms = |to: &str, body: &str| SmsDraft {
            to: to.into(),
            body: body.into(),
        };
        assert_eq!(sms("+33612345678", "salut").problems(), []);
        assert_eq!(
            sms("x", " ").problems(),
            [("to", Problem::Invalid), ("body", Problem::Required)]
        );
        assert_eq!(
            sms("+33612345678", &"x".repeat(1601)).problems(),
            [("body", Problem::TooLong { max: 1600 })]
        );
    }

    #[test]
    fn what_the_kotlin_app_sends_is_read_with_the_reference_defaults() {
        let sms = parse_device_frame(
            r#"{"type":"sms_received","from":"+33612345678","body":"Salut","contact_name":"Alice","timestamp":"2026-01-02T10:00:00+01:00"}"#,
        );
        assert_eq!(
            sms,
            Some(DeviceFrame::SmsReceived(IncomingSms {
                from: "+33612345678".into(),
                contact_name: "Alice".into(),
                body: "Salut".into(),
                timestamp: Some(Utc.with_ymd_and_hms(2026, 1, 2, 9, 0, 0).unwrap()),
            }))
        );
        // Champs absents ou illisibles : valeurs par defaut de la reference.
        let Some(DeviceFrame::SmsReceived(s)) =
            parse_device_frame(r#"{"type":"sms_received","timestamp":"hier"}"#)
        else {
            panic!()
        };
        assert_eq!(
            (s.from.as_str(), s.body.as_str(), s.timestamp),
            ("", "", None)
        );
        let Some(DeviceFrame::CallEvent(c)) =
            parse_device_frame(r#"{"type":"call_event","number":"0612"}"#)
        else {
            panic!()
        };
        assert_eq!(
            (c.state, c.duration),
            (CallState::Ringing, 0),
            "etat par defaut : ringing"
        );
        let Some(DeviceFrame::CallEvent(c)) = parse_device_frame(
            r#"{"type":"call_event","state":"ended","number":"0612","duration":75.0}"#,
        ) else {
            panic!()
        };
        assert_eq!((c.state, c.duration), (CallState::Ended, 75));
        assert_eq!(
            parse_device_frame(
                r#"{"type":"sms_sent_ack","message_id":42,"success":true,"error":""}"#
            ),
            Some(DeviceFrame::SmsSentAck(SmsAck {
                message_id: Some(42),
                success: true,
                error: String::new()
            }))
        );
        let Some(DeviceFrame::SmsSentAck(a)) =
            parse_device_frame(r#"{"type":"sms_sent_ack","message_id":0}"#)
        else {
            panic!()
        };
        assert_eq!(
            (a.message_id, a.success),
            (None, false),
            "0 : rien a rapprocher"
        );
        assert_eq!(
            parse_device_frame(r#"{"type":"ping"}"#),
            Some(DeviceFrame::Ping)
        );
        assert_eq!(
            parse_device_frame(r#"{"type":"inconnu"}"#),
            Some(DeviceFrame::Other)
        );
        assert_eq!(
            parse_device_frame(r#"{"sans":"type"}"#),
            Some(DeviceFrame::Other)
        );
        for bad in ["pas du json", "[1]", "42", "null", ""] {
            assert_eq!(parse_device_frame(bad), None, "{bad}");
        }
    }

    #[test]
    fn what_the_server_sends_has_the_reference_keys() {
        let j = |f: ServerFrame| serde_json::from_str::<Value>(&f.to_json()).unwrap();
        assert_eq!(
            j(ServerFrame::AuthOk {
                device_name: "Pixel".into()
            }),
            json!({"type": "auth_ok", "device_name": "Pixel"})
        );
        assert_eq!(
            j(ServerFrame::SendSms {
                to: "+33612345678".into(),
                body: "Coucou".into(),
                message_id: 7
            }),
            json!({"type": "send_sms", "to": "+33612345678", "body": "Coucou", "message_id": 7})
        );
        assert_eq!(j(ServerFrame::Pong), json!({"type": "pong"}));
        assert_eq!(
            j(ServerFrame::AlertNotification {
                alert_id: 3,
                rule_name: "Canicule".into(),
                severity: "critical".into(),
                message: "[DomiNet] Canicule".into()
            }),
            json!({"type": "alert_notification", "alert_id": 3, "rule_name": "Canicule", "severity": "critical", "message": "[DomiNet] Canicule"})
        );
    }

    #[test]
    fn the_call_log_follows_the_lifecycle_of_reference() {
        let dev = AndroidDeviceId(1);
        let open = CallLog {
            id: CallId(9),
            device: dev,
            direction: CallDirection::Incoming,
            phone_number: "0612".into(),
            contact_name: String::new(),
            started_at: at(10, 0),
            ended_at: None,
            duration: 0,
            received_at: at(10, 0),
        };
        // ringing / outgoing creent ; la date manquante devient l'instant de reception.
        let CallPlan::Create(c) =
            plan_call(dev, &event("ringing", "0612", None, 0), None, at(10, 0))
        else {
            panic!()
        };
        assert_eq!(
            (c.direction, c.started_at, c.ended_at),
            (CallDirection::Incoming, at(10, 0), None)
        );
        let CallPlan::Create(c) = plan_call(
            dev,
            &event("outgoing", "0612", Some(at(11, 0)), 0),
            None,
            at(10, 0),
        ) else {
            panic!()
        };
        assert_eq!(
            (c.direction, c.started_at),
            (CallDirection::Outgoing, at(11, 0))
        );
        // ended ferme l'appel ouvert et garde la duree ; missed le passe en manque.
        assert_eq!(
            plan_call(
                dev,
                &event("ended", "0612", Some(at(10, 5)), 300),
                Some(&open),
                at(10, 6)
            ),
            CallPlan::Update {
                id: CallId(9),
                direction: None,
                ended_at: at(10, 5),
                duration: Some(300)
            }
        );
        assert_eq!(
            plan_call(
                dev,
                &event("missed", "0612", Some(at(10, 1)), 0),
                Some(&open),
                at(10, 6)
            ),
            CallPlan::Update {
                id: CallId(9),
                direction: Some(CallDirection::Missed),
                ended_at: at(10, 1),
                duration: None
            }
        );
        // Sans appel ouvert, missed / ended creent une entree deja terminee.
        let CallPlan::Create(c) = plan_call(
            dev,
            &event("missed", "0699", Some(at(12, 0)), 0),
            None,
            at(12, 1),
        ) else {
            panic!()
        };
        assert_eq!(
            (c.direction, c.ended_at, c.duration),
            (CallDirection::Missed, Some(at(12, 0)), 0)
        );
        let CallPlan::Create(c) = plan_call(
            dev,
            &event("ended", "0699", Some(at(12, 0)), 42),
            None,
            at(12, 1),
        ) else {
            panic!()
        };
        assert_eq!((c.direction, c.duration), (CallDirection::Incoming, 42));
        // answered et les etats inconnus ne font rien.
        for state in ["answered", "inconnu"] {
            assert_eq!(
                plan_call(dev, &event(state, "0612", None, 0), Some(&open), at(10, 6)),
                CallPlan::Nothing,
                "{state}"
            );
        }
        assert_eq!(open.duration_display(), "00:00");
    }
}
