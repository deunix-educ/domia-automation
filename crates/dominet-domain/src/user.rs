// SPDX-License-Identifier: AGPL-3.0-or-later
//! User accounts.

use std::fmt;
use std::str::FromStr;

use chrono::{DateTime, Utc};

use crate::error::{Problem, ValidationError};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct UserId(pub i64);

impl fmt::Display for UserId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

/// Role of an account: `admin` manages everything, `user` views and commands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Role {
    Admin,
    User,
}

impl Role {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Admin => "admin",
            Self::User => "user",
        }
    }

    #[must_use]
    pub fn is_admin(self) -> bool {
        self == Self::Admin
    }
}

impl FromStr for Role {
    type Err = ValidationError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "admin" => Ok(Self::Admin),
            "user" => Ok(Self::User),
            other => Err(ValidationError::new(format!(
                "unknown role: \"{other}\" (admin or user)"
            ))),
        }
    }
}

/// Valid username: 1 to 150 characters, letters, digits or `@ . + - _`, no space.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Username(String);

impl Username {
    /// # Errors
    /// Refuses an empty name, one that is too long or contains a disallowed character.
    pub fn parse(raw: &str) -> Result<Self, ValidationError> {
        let n = raw.chars().count();
        if n == 0 || n > 150 {
            return Err(ValidationError::new(
                "the user name must be 1 to 150 characters long",
            ));
        }
        if let Some(bad) = raw
            .chars()
            .find(|c| !(c.is_alphanumeric() || "@.+-_".contains(*c)))
        {
            return Err(ValidationError::new(format!(
                "forbidden character in the user name: \"{bad}\" (letters, digits and @ . + - _ only)"
            )));
        }
        Ok(Self(raw.to_string()))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for Username {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Account fields entered in the administration, to check before writing.
#[derive(Debug, Clone, Copy)]
pub struct AccountFields<'a> {
    pub email: &'a str,
    pub first_name: &'a str,
    pub last_name: &'a str,
    pub mobile: &'a str,
}

impl AccountFields<'_> {
    pub const EMAIL_MAX: usize = 254;
    pub const NAME_MAX: usize = 150;

    /// Field-by-field problems (`email`, `first_name`, `last_name`, `mobile`).
    #[must_use]
    pub fn problems(&self) -> Vec<(&'static str, Problem)> {
        let mut out = vec![];
        let email = self.email.trim();
        if email.chars().count() > Self::EMAIL_MAX {
            out.push((
                "email",
                Problem::TooLong {
                    max: Self::EMAIL_MAX,
                },
            ));
        } else if !email.is_empty() {
            let ok = email.split_once('@').is_some_and(|(local, domain)| {
                !local.is_empty()
                    && domain.contains('.')
                    && !domain.starts_with('.')
                    && !domain.ends_with('.')
            }) && !email.contains(char::is_whitespace)
                && email.matches('@').count() == 1;
            if !ok {
                out.push(("email", Problem::Invalid));
            }
        }
        for (field, value) in [
            ("first_name", self.first_name),
            ("last_name", self.last_name),
        ] {
            if value.trim().chars().count() > Self::NAME_MAX {
                out.push((
                    field,
                    Problem::TooLong {
                        max: Self::NAME_MAX,
                    },
                ));
            }
        }
        if normalize_mobile(self.mobile).is_err() {
            out.push(("mobile", Problem::Invalid));
        }
        out
    }
}

impl Username {
    /// Reason why `raw` is not a valid username (`None` if it is).
    #[must_use]
    pub fn problem(raw: &str) -> Option<Problem> {
        let n = raw.chars().count();
        if n == 0 {
            Some(Problem::Required)
        } else if n > 150 {
            Some(Problem::TooLong { max: 150 })
        } else if Self::parse(raw).is_err() {
            Some(Problem::Invalid)
        } else {
            None
        }
    }
}

/// Normalises a mobile number (e.g. `+33 6 12 34 56 78` -> `+33612345678`); empty is accepted.
///
/// # Errors
/// Refuses anything that is not an optional `+` followed by 6 to 19 digits.
pub fn normalize_mobile(raw: &str) -> Result<String, ValidationError> {
    let cleaned: String = raw
        .chars()
        .filter(|c| !matches!(c, ' ' | '.' | '-'))
        .collect();
    if cleaned.is_empty() {
        return Ok(cleaned);
    }
    let digits = cleaned.strip_prefix('+').unwrap_or(&cleaned);
    if !(6..=19).contains(&digits.len()) || !digits.chars().all(|c| c.is_ascii_digit()) {
        return Err(ValidationError::new(
            "invalid mobile number (e.g. +33612345678)",
        ));
    }
    Ok(cleaned)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct User {
    pub id: UserId,
    pub username: String,
    pub email: String,
    pub first_name: String,
    pub last_name: String,
    pub mobile: String,
    pub role: Role,
    pub is_active: bool,
    pub last_login_at: Option<DateTime<Utc>>,
    pub password_changed_at: DateTime<Utc>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl User {
    /// First name + last name if they exist, otherwise the username (like `User.__str__` in the reference).
    #[must_use]
    pub fn display_name(&self) -> String {
        let full = format!("{} {}", self.first_name.trim(), self.last_name.trim());
        let full = full.trim();
        if full.is_empty() {
            self.username.clone()
        } else {
            full.to_string()
        }
    }
}

/// Data of an account to create; the password arrives already hashed (argon2id).
#[derive(Clone)]
pub struct NewUser {
    pub username: Username,
    pub email: String,
    pub first_name: String,
    pub last_name: String,
    pub mobile: String,
    pub role: Role,
    pub is_active: bool,
    pub password_hash: String,
}

impl fmt::Debug for NewUser {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NewUser")
            .field("username", &self.username)
            .field("role", &self.role)
            .finish_non_exhaustive()
    }
}

/// Editable fields of an account (the username and the password have their own operations).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UserProfile {
    pub email: String,
    pub first_name: String,
    pub last_name: String,
    pub mobile: String,
    pub role: Role,
    pub is_active: bool,
}

/// Account + password hash, for verification at login only.
#[derive(Clone)]
pub struct Credentials {
    pub user: User,
    pub password_hash: String,
}

impl fmt::Debug for Credentials {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Credentials")
            .field("user", &self.user.username)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn username_accepts_the_reference_alphabet() {
        for ok in [
            "bob",
            "Bob.Martin",
            "a+b@c-d_e",
            "élodie",
            "x".repeat(150).as_str(),
        ] {
            assert!(Username::parse(ok).is_ok(), "{ok}");
        }
    }

    #[test]
    fn username_rejects_bad_values() {
        for bad in [
            "",
            "a b",
            "bob/",
            "x\ny",
            "a;drop",
            "<script>",
            &"x".repeat(151),
        ] {
            assert!(Username::parse(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn mobile_is_normalized_and_validated() {
        assert_eq!(
            normalize_mobile("+33 6 12.34-56 78").unwrap(),
            "+33612345678"
        );
        assert_eq!(normalize_mobile("0612345678").unwrap(), "0612345678");
        assert_eq!(normalize_mobile("").unwrap(), "");
        for bad in [
            "+33abc",
            "12",
            "+",
            "++33612345678",
            "+3361234567890123456789",
        ] {
            assert!(normalize_mobile(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn role_round_trips() {
        for r in [Role::Admin, Role::User] {
            assert_eq!(r.as_str().parse::<Role>().unwrap(), r);
        }
        assert!("root".parse::<Role>().is_err());
        assert!(Role::Admin.is_admin() && !Role::User.is_admin());
    }

    fn user(first: &str, last: &str) -> User {
        let now = Utc::now();
        User {
            id: UserId(1),
            username: "bob".into(),
            email: String::new(),
            first_name: first.into(),
            last_name: last.into(),
            mobile: String::new(),
            role: Role::User,
            is_active: true,
            last_login_at: None,
            password_changed_at: now,
            created_at: now,
            updated_at: now,
        }
    }

    #[test]
    fn display_name_falls_back_to_username() {
        assert_eq!(user("Alice", "Martin").display_name(), "Alice Martin");
        assert_eq!(user("Alice", "").display_name(), "Alice");
        assert_eq!(user("", "").display_name(), "bob");
        assert_eq!(user("  ", " ").display_name(), "bob");
    }

    #[test]
    fn password_hashes_never_appear_in_debug() {
        let c = Credentials {
            user: user("", ""),
            password_hash: "$argon2id$secret".into(),
        };
        assert!(!format!("{c:?}").contains("argon2id"));
        let n = NewUser {
            username: Username::parse("bob").unwrap(),
            email: String::new(),
            first_name: String::new(),
            last_name: String::new(),
            mobile: String::new(),
            role: Role::User,
            is_active: true,
            password_hash: "$argon2id$secret".into(),
        };
        assert!(!format!("{n:?}").contains("argon2id"));
    }

    #[test]
    fn account_fields_report_problems_per_field() {
        let ok = AccountFields {
            email: "bob@example.org",
            first_name: "Bob",
            last_name: "",
            mobile: "+33 6 12 34 56 78",
        };
        assert_eq!(ok.problems().len(), 0);
        let empty = AccountFields {
            email: "",
            first_name: "",
            last_name: "",
            mobile: "",
        };
        assert_eq!(empty.problems().len(), 0, "tout est facultatif");
        for bad in [
            "bob",
            "@x.org",
            "bob@",
            "bob@@x.org",
            "a b@x.org",
            "bob@x",
            "bob@.org",
        ] {
            let f = AccountFields {
                email: bad,
                ..empty
            };
            assert_eq!(f.problems(), [("email", Problem::Invalid)], "{bad}");
        }
        let long = AccountFields {
            email: &format!("{}@x.org", "a".repeat(250)),
            first_name: &"x".repeat(151),
            last_name: &"y".repeat(151),
            mobile: "12ab",
        };
        assert_eq!(
            long.problems(),
            [
                ("email", Problem::TooLong { max: 254 }),
                ("first_name", Problem::TooLong { max: 150 }),
                ("last_name", Problem::TooLong { max: 150 }),
                ("mobile", Problem::Invalid),
            ]
        );
    }

    #[test]
    fn username_problems_are_classified() {
        assert_eq!(Username::problem("bob.martin@home"), None);
        assert_eq!(Username::problem(""), Some(Problem::Required));
        assert_eq!(
            Username::problem(&"x".repeat(151)),
            Some(Problem::TooLong { max: 150 })
        );
        assert_eq!(Username::problem("bob martin"), Some(Problem::Invalid));
        assert_eq!(Username::problem("bob/../x"), Some(Problem::Invalid));
    }
}
