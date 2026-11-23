// SPDX-License-Identifier: AGPL-3.0-or-later
//! Physical locations (rooms, zones).

use crate::error::{Problem, ValidationError};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct LocationId(pub i64);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Location {
    pub id: LocationId,
    pub name: String,
    pub description: String,
    pub active: bool,
}

/// Location to create or replace.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocationDraft {
    pub name: String,
    pub description: String,
    pub active: bool,
}

impl LocationDraft {
    pub const NAME_MAX: usize = 64;
    pub const DESCRIPTION_MAX: usize = 128;

    /// All the problems, field by field (`validate` only reports the first one, in French).
    #[must_use]
    pub fn problems(&self) -> Vec<(&'static str, Problem)> {
        let mut out = vec![];
        if self.name.trim().is_empty() {
            out.push(("name", Problem::Required));
        } else if self.name.chars().count() > Self::NAME_MAX {
            out.push((
                "name",
                Problem::TooLong {
                    max: Self::NAME_MAX,
                },
            ));
        }
        if self.description.chars().count() > Self::DESCRIPTION_MAX {
            out.push((
                "description",
                Problem::TooLong {
                    max: Self::DESCRIPTION_MAX,
                },
            ));
        }
        out
    }

    /// # Errors
    /// Name of 1 to 64 characters, description of 128 characters at most.
    pub fn validate(&self) -> Result<(), ValidationError> {
        if self.name.trim().is_empty() || self.name.chars().count() > 64 {
            return Err(ValidationError::new("name: 1 to 64 characters"));
        }
        if self.description.chars().count() > 128 {
            return Err(ValidationError::new("description: 128 characters at most"));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn problems_are_reported_per_field_and_agree_with_validate() {
        let ok = LocationDraft {
            name: "Salon".into(),
            description: String::new(),
            active: true,
        };
        assert_eq!(ok.problems().len(), 0);
        let bad = LocationDraft {
            name: " ".into(),
            description: "x".repeat(129),
            active: true,
        };
        assert_eq!(
            bad.problems(),
            [
                ("name", Problem::Required),
                ("description", Problem::TooLong { max: 128 })
            ]
        );
        let long = LocationDraft {
            name: "x".repeat(65),
            ..ok.clone()
        };
        assert_eq!(long.problems(), [("name", Problem::TooLong { max: 64 })]);
        for d in [&ok, &bad, &long] {
            assert_eq!(d.validate().is_ok(), d.problems().is_empty());
        }
    }

    #[test]
    fn name_and_description_limits() {
        let ok = LocationDraft {
            name: "Salon".into(),
            description: String::new(),
            active: true,
        };
        assert!(ok.validate().is_ok());
        assert!(
            LocationDraft {
                name: " ".into(),
                ..ok.clone()
            }
            .validate()
            .is_err()
        );
        assert!(
            LocationDraft {
                name: "x".repeat(65),
                ..ok.clone()
            }
            .validate()
            .is_err()
        );
        assert!(
            LocationDraft {
                description: "x".repeat(129),
                ..ok.clone()
            }
            .validate()
            .is_err()
        );
        assert!(
            LocationDraft {
                name: "é".repeat(64),
                description: "é".repeat(128),
                active: false
            }
            .validate()
            .is_ok()
        );
    }
}
