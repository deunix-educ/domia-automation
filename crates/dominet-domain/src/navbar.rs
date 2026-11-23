// SPDX-License-Identifier: AGPL-3.0-or-later
//! Navigation entries.

use std::str::FromStr;

use crate::error::{Problem, ValidationError};
use crate::user::Role;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct NavbarItemId(pub i64);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NavPosition {
    Top,
    Bottom,
}

impl NavPosition {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Top => "top",
            Self::Bottom => "bottom",
        }
    }
}

impl FromStr for NavPosition {
    type Err = ValidationError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "top" => Ok(Self::Top),
            "bottom" => Ok(Self::Bottom),
            other => Err(ValidationError::new(format!(
                "unknown position: \"{other}\" (top or bottom)"
            ))),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NavbarItem {
    pub id: NavbarItemId,
    pub icon_name: String,
    pub label: String,
    pub url: String,
    pub url_target: String,
    pub admin_only: bool,
    pub position: NavPosition,
    pub sort_order: i16,
}

impl NavbarItem {
    /// An entry reserved to administrators is only shown to them.
    #[must_use]
    pub fn visible_to(&self, role: Role) -> bool {
        !self.admin_only || role.is_admin()
    }
}

/// Entry to create or replace.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NavbarDraft {
    pub icon_name: String,
    pub label: String,
    pub url: String,
    pub url_target: String,
    pub admin_only: bool,
    pub position: NavPosition,
    pub sort_order: i16,
}

impl NavbarDraft {
    pub const LABEL_MAX: usize = 128;
    pub const URL_MAX: usize = 255;
    pub const ICON_MAX: usize = 64;

    fn icon_name_is_valid(&self) -> bool {
        self.icon_name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
    }

    fn url_is_valid(&self) -> bool {
        !self.url.contains(char::is_whitespace)
            && ((self.url.starts_with('/') && !self.url.starts_with("//"))
                || self.url.starts_with("http://")
                || self.url.starts_with("https://"))
    }

    /// All the problems, field by field (`validate` only reports the first one, in French). The fields are
    /// `icon_name`, `label`, `url`, `url_target` and `sort_order`.
    #[must_use]
    pub fn problems(&self) -> Vec<(&'static str, Problem)> {
        let mut out = vec![];
        if self.icon_name.is_empty() {
            out.push(("icon_name", Problem::Required));
        } else if self.icon_name.len() > Self::ICON_MAX {
            out.push((
                "icon_name",
                Problem::TooLong {
                    max: Self::ICON_MAX,
                },
            ));
        } else if !self.icon_name_is_valid() {
            out.push(("icon_name", Problem::Invalid));
        }
        if self.label.trim().is_empty() {
            out.push(("label", Problem::Required));
        } else if self.label.chars().count() > Self::LABEL_MAX {
            out.push((
                "label",
                Problem::TooLong {
                    max: Self::LABEL_MAX,
                },
            ));
        }
        if self.url.is_empty() {
            out.push(("url", Problem::Required));
        } else if self.url.len() > Self::URL_MAX {
            out.push(("url", Problem::TooLong { max: Self::URL_MAX }));
        } else if !self.url_is_valid() {
            out.push(("url", Problem::Invalid));
        }
        if !matches!(self.url_target.as_str(), "_self" | "_blank") {
            out.push(("url_target", Problem::Invalid));
        }
        if self.sort_order < 0 {
            out.push((
                "sort_order",
                Problem::OutOfRange {
                    min: 0,
                    max: i64::from(i16::MAX),
                },
            ));
        }
        out
    }

    /// Rules: Lucide icon in kebab-case, label 1-128, internal URL (`/...`) or `http(s)://` only
    /// (never `javascript:` or `data:`), target `_self` or `_blank`, positive order.
    ///
    /// # Errors
    /// Returns the first rule violated.
    pub fn validate(&self) -> Result<(), ValidationError> {
        let icon_ok = !self.icon_name.is_empty()
            && self.icon_name.len() <= 64
            && self
                .icon_name
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-');
        if !icon_ok {
            return Err(ValidationError::new(
                "icon: Lucide name in lowercase with dashes (e.g. wifi-sync)",
            ));
        }
        if self.label.trim().is_empty() || self.label.chars().count() > 128 {
            return Err(ValidationError::new("label: 1 to 128 characters"));
        }
        let url_ok = self.url.len() <= 255
            && !self.url.contains(char::is_whitespace)
            && ((self.url.starts_with('/') && !self.url.starts_with("//"))
                || self.url.starts_with("http://")
                || self.url.starts_with("https://"));
        if !url_ok {
            return Err(ValidationError::new(
                "URL: internal path (/...) or http(s):// address without spaces, 255 characters at most",
            ));
        }
        if !matches!(self.url_target.as_str(), "_self" | "_blank") {
            return Err(ValidationError::new("target: _self or _blank"));
        }
        if self.sort_order < 0 {
            return Err(ValidationError::new("order: non-negative integer"));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn draft() -> NavbarDraft {
        NavbarDraft {
            icon_name: "video".into(),
            label: "Surveillance".into(),
            url: "/surveillance/".into(),
            url_target: "_self".into(),
            admin_only: false,
            position: NavPosition::Top,
            sort_order: 1,
        }
    }

    #[test]
    fn problems_are_per_field_and_agree_with_validate() {
        assert_eq!(draft().problems().len(), 0);
        let bad = NavbarDraft {
            icon_name: "Video!".into(),
            label: " ".into(),
            url: "javascript:alert(1)".into(),
            url_target: "_top".into(),
            sort_order: -1,
            ..draft()
        };
        assert_eq!(
            bad.problems(),
            [
                ("icon_name", Problem::Invalid),
                ("label", Problem::Required),
                ("url", Problem::Invalid),
                ("url_target", Problem::Invalid),
                ("sort_order", Problem::OutOfRange { min: 0, max: 32767 }),
            ]
        );
        let long = NavbarDraft {
            url: format!("/{}", "x".repeat(255)),
            label: "x".repeat(129),
            ..draft()
        };
        assert_eq!(
            long.problems(),
            [
                ("label", Problem::TooLong { max: 128 }),
                ("url", Problem::TooLong { max: 255 })
            ]
        );
        let empty = NavbarDraft {
            icon_name: String::new(),
            url: String::new(),
            ..draft()
        };
        assert_eq!(
            empty.problems(),
            [("icon_name", Problem::Required), ("url", Problem::Required)]
        );
        for d in [draft(), bad, long, empty] {
            assert_eq!(d.validate().is_ok(), d.problems().is_empty());
        }
    }

    #[test]
    fn a_normal_entry_is_valid() {
        assert!(draft().validate().is_ok());
        let ext = NavbarDraft {
            url: "http://dominet.local:8080".into(),
            url_target: "_blank".into(),
            ..draft()
        };
        assert!(ext.validate().is_ok());
    }

    #[test]
    fn dangerous_or_malformed_urls_are_rejected() {
        for bad in [
            "javascript:alert(1)",
            "data:text/html,x",
            "//evil.example/x",
            "surveillance",
            "ftp://x",
            "/a b",
            "",
        ] {
            let d = NavbarDraft {
                url: bad.into(),
                ..draft()
            };
            assert!(d.validate().is_err(), "{bad:?}");
        }
    }

    #[test]
    fn icon_labels_targets_and_order_are_checked() {
        let cases = [
            NavbarDraft {
                icon_name: "Zigbee2Mqtt".into(),
                ..draft()
            },
            NavbarDraft {
                icon_name: String::new(),
                ..draft()
            },
            NavbarDraft {
                label: "  ".into(),
                ..draft()
            },
            NavbarDraft {
                url_target: "_top".into(),
                ..draft()
            },
            NavbarDraft {
                sort_order: -1,
                ..draft()
            },
        ];
        for c in cases {
            assert!(c.validate().is_err(), "{c:?}");
        }
    }

    #[test]
    fn admin_only_entries_are_hidden_from_users() {
        let item = |admin_only| NavbarItem {
            id: NavbarItemId(1),
            icon_name: "x".into(),
            label: "x".into(),
            url: "/".into(),
            url_target: "_self".into(),
            admin_only,
            position: NavPosition::Bottom,
            sort_order: 0,
        };
        assert!(item(false).visible_to(Role::User));
        assert!(!item(true).visible_to(Role::User));
        assert!(item(true).visible_to(Role::Admin));
    }
}
