// SPDX-License-Identifier: AGPL-3.0-or-later
//! Scenes (R4, ADR 0031): a set of commands applied together to several devices.
//!
//! Activating a scene = publishing each command on its device's command topic, **in order**, at QoS 1. A failing action does not stop
//! the following ones (like the reference implementation): the errors are reported at the end.

use serde_json::Value;

use crate::command::check_command;
use crate::device::DeviceId;
use crate::error::Problem;
use crate::location::LocationId;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SceneId(pub i64);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SceneActionId(pub i64);

/// At most this many actions per scene (safeguard, the reference did not limit them).
pub const MAX_SCENE_ACTIONS: usize = 100;

#[derive(Debug, Clone, PartialEq)]
pub struct SceneAction {
    pub id: SceneActionId,
    pub device: DeviceId,
    pub payload: Value,
    pub position: i16,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Scene {
    pub id: SceneId,
    pub name: String,
    pub icon: String,
    pub description: String,
    pub location: Option<LocationId>,
    pub active: bool,
    /// In publication order.
    pub actions: Vec<SceneAction>,
}

/// A command to save (its position is its rank in the list).
#[derive(Debug, Clone, PartialEq)]
pub struct SceneActionDraft {
    pub device: DeviceId,
    pub payload: Value,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SceneDraft {
    pub name: String,
    pub icon: String,
    pub description: String,
    pub location: Option<LocationId>,
    pub active: bool,
    pub actions: Vec<SceneActionDraft>,
}

impl SceneDraft {
    #[must_use]
    pub fn new(name: &str) -> Self {
        Self {
            name: name.to_string(),
            icon: String::new(),
            description: String::new(),
            location: None,
            active: true,
            actions: vec![],
        }
    }

    /// Field-by-field problems. `actions`: too many, or a command that is not an allowed JSON object.
    #[must_use]
    pub fn problems(&self) -> Vec<(&'static str, Problem)> {
        let mut out = vec![];
        if self.name.trim().is_empty() {
            out.push(("name", Problem::Required));
        } else if self.name.chars().count() > 64 {
            out.push(("name", Problem::TooLong { max: 64 }));
        }
        if self.icon.chars().count() > 32 {
            out.push(("icon", Problem::TooLong { max: 32 }));
        }
        if self.description.chars().count() > 256 {
            out.push(("description", Problem::TooLong { max: 256 }));
        }
        if self.actions.len() > MAX_SCENE_ACTIONS
            || self
                .actions
                .iter()
                .any(|a| check_command(&a.payload).is_err())
        {
            out.push(("actions", Problem::Invalid));
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn action(payload: Value) -> SceneActionDraft {
        SceneActionDraft {
            device: DeviceId(1),
            payload,
        }
    }

    #[test]
    fn a_scene_needs_a_name_and_valid_commands() {
        let mut s = SceneDraft::new("Fermeture du soir");
        s.actions = vec![
            action(json!({"state": "CLOSE"})),
            action(json!({"state": "OFF"})),
        ];
        assert_eq!(s.problems(), []);
        assert!(
            SceneDraft::new("Vide").problems().is_empty(),
            "une scene sans action est admise (comme la reference)"
        );
        assert_eq!(
            SceneDraft::new(" ").problems(),
            [("name", Problem::Required)]
        );
        assert_eq!(
            SceneDraft::new(&"x".repeat(65)).problems(),
            [("name", Problem::TooLong { max: 64 })]
        );
        for bad in [
            json!([1]),
            json!("ON"),
            json!({}),
            json!({"a": {"b": {"c": {"d": 1}}}}),
        ] {
            let mut s = SceneDraft::new("X");
            s.actions = vec![action(bad.clone())];
            assert_eq!(s.problems(), [("actions", Problem::Invalid)], "{bad}");
        }
        let mut many = SceneDraft::new("X");
        many.actions = (0..=MAX_SCENE_ACTIONS)
            .map(|_| action(json!({"state": "ON"})))
            .collect();
        assert_eq!(many.problems(), [("actions", Problem::Invalid)]);
    }
}
