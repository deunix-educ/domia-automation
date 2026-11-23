// SPDX-License-Identifier: AGPL-3.0-or-later
//! Internal messaging (R5, ADR 0037): a message to several users, by e-mail, SMS or both.

use chrono::{DateTime, Utc};

use crate::error::Problem;
use crate::user::UserId;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct MessageId(pub i64);

/// A recipient of a message (`message_recipient` row): also serves as the origin of queued SMS.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct DeliveryId(pub i64);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Channel {
    Email,
    Sms,
    Both,
}

impl Channel {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Email => "email",
            Self::Sms => "sms",
            Self::Both => "both",
        }
    }

    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        [Self::Email, Self::Sms, Self::Both]
            .into_iter()
            .find(|c| c.as_str() == s)
    }

    #[must_use]
    pub fn uses_email(self) -> bool {
        matches!(self, Self::Email | Self::Both)
    }

    #[must_use]
    pub fn uses_sms(self) -> bool {
        matches!(self, Self::Sms | Self::Both)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MessageStatus {
    Draft,
    Sending,
    Sent,
}

impl MessageStatus {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Draft => "draft",
            Self::Sending => "sending",
            Self::Sent => "sent",
        }
    }

    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        [Self::Draft, Self::Sending, Self::Sent]
            .into_iter()
            .find(|c| c.as_str() == s)
    }
}

/// State of a sending (e-mail or SMS) for a recipient.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeliveryStatus {
    Pending,
    Sent,
    Failed,
    /// Channel not requested, or recipient without an address / without a mobile.
    NotApplicable,
}

impl DeliveryStatus {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Sent => "sent",
            Self::Failed => "failed",
            Self::NotApplicable => "na",
        }
    }

    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        [Self::Pending, Self::Sent, Self::Failed, Self::NotApplicable]
            .into_iter()
            .find(|c| c.as_str() == s)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Message {
    pub id: MessageId,
    pub sender: UserId,
    pub subject: String,
    pub body: String,
    pub channel: Channel,
    pub status: MessageStatus,
    pub created_at: DateTime<Utc>,
    pub sent_at: Option<DateTime<Utc>>,
    pub recipients_count: i32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Delivery {
    pub id: DeliveryId,
    pub message: MessageId,
    pub recipient: UserId,
    pub email: DeliveryStatus,
    pub sms: DeliveryStatus,
    pub error: String,
    pub updated_at: DateTime<Utc>,
}

/// What the user enters to write a message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MessageDraft {
    pub subject: String,
    pub body: String,
    pub channel: Channel,
    pub recipients: Vec<UserId>,
}

impl MessageDraft {
    /// Field-by-field problems (`subject`, `body`, `recipients`).
    #[must_use]
    pub fn problems(&self) -> Vec<(&'static str, Problem)> {
        let mut out = vec![];
        if self.subject.trim().is_empty() {
            out.push(("subject", Problem::Required));
        } else if self.subject.chars().count() > 200 {
            out.push(("subject", Problem::TooLong { max: 200 }));
        }
        if self.body.trim().is_empty() {
            out.push(("body", Problem::Required));
        }
        if self.recipients.is_empty() {
            out.push(("recipients", Problem::Required));
        }
        out
    }
}

/// Initial states of a recipient according to the channel and what they have (`(email, sms)`): `pending` only if the channel is requested
/// **and** the contact detail exists, otherwise `na`.
#[must_use]
pub fn initial_statuses(
    channel: Channel,
    has_email: bool,
    has_mobile: bool,
) -> (DeliveryStatus, DeliveryStatus) {
    let pick = |wanted: bool, has: bool| {
        if wanted && has {
            DeliveryStatus::Pending
        } else {
            DeliveryStatus::NotApplicable
        }
    };
    (
        pick(channel.uses_email(), has_email),
        pick(channel.uses_sms(), has_mobile),
    )
}

/// Final state of a message: `sent` when no sending is pending any more.
#[must_use]
pub fn is_complete(deliveries: &[Delivery]) -> bool {
    deliveries
        .iter()
        .all(|d| d.email != DeliveryStatus::Pending && d.sms != DeliveryStatus::Pending)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enums_round_trip() {
        for c in [Channel::Email, Channel::Sms, Channel::Both] {
            assert_eq!(Channel::parse(c.as_str()), Some(c));
        }
        for s in [
            MessageStatus::Draft,
            MessageStatus::Sending,
            MessageStatus::Sent,
        ] {
            assert_eq!(MessageStatus::parse(s.as_str()), Some(s));
        }
        for s in [
            DeliveryStatus::Pending,
            DeliveryStatus::Sent,
            DeliveryStatus::Failed,
            DeliveryStatus::NotApplicable,
        ] {
            assert_eq!(DeliveryStatus::parse(s.as_str()), Some(s));
        }
        assert_eq!(Channel::parse("fax"), None);
    }

    #[test]
    fn draft_is_validated_field_by_field() {
        let d = |s: &str, b: &str, r: Vec<UserId>| MessageDraft {
            subject: s.into(),
            body: b.into(),
            channel: Channel::Email,
            recipients: r,
        };
        assert_eq!(d("Sujet", "Corps", vec![UserId(1)]).problems(), []);
        assert_eq!(
            d(" ", "", vec![]).problems(),
            [
                ("subject", Problem::Required),
                ("body", Problem::Required),
                ("recipients", Problem::Required)
            ]
        );
        assert_eq!(
            d(&"x".repeat(201), "c", vec![UserId(1)]).problems(),
            [("subject", Problem::TooLong { max: 200 })]
        );
    }

    #[test]
    fn initial_statuses_need_channel_and_contact() {
        use DeliveryStatus::{NotApplicable as Na, Pending as P};
        assert_eq!(initial_statuses(Channel::Email, true, true), (P, Na));
        assert_eq!(initial_statuses(Channel::Sms, true, true), (Na, P));
        assert_eq!(initial_statuses(Channel::Both, true, false), (P, Na));
        assert_eq!(initial_statuses(Channel::Both, false, true), (Na, P));
        assert_eq!(initial_statuses(Channel::Email, false, true), (Na, Na));
    }

    #[test]
    fn message_is_complete_when_nothing_is_pending() {
        use DeliveryStatus::{Failed, NotApplicable as Na, Pending, Sent};
        let d = |e, s| Delivery {
            id: DeliveryId(1),
            message: MessageId(1),
            recipient: UserId(1),
            email: e,
            sms: s,
            error: String::new(),
            updated_at: Utc::now(),
        };
        assert!(is_complete(&[d(Sent, Na), d(Failed, Sent)]));
        assert!(!is_complete(&[d(Sent, Na), d(Sent, Pending)]));
        assert!(is_complete(&[]));
    }
}
