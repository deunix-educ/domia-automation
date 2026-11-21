// SPDX-License-Identifier: AGPL-3.0-or-later
//! Changes made through the assistant, and how to take them back (R12 lot A3).
//!
//! Before a confirmed card changes the configuration, the item is read (`config_get`) and what it takes to put it back is written
//! to the audit trail **before** the change: a replacement is undone by writing the old item again, a creation by deleting the new
//! item, a deletion by creating the item again. Nothing else is kept in memory: the audit trail is the one record, so an undo
//! survives a restart (until the trail is purged).
//!
//! What cannot be taken back is said, not hidden: a device command or a scene already happened; the code of a Rhai script is
//! never readable through the tools, so a change that involves code has no snapshot to restore.

use chrono::{DateTime, Utc};
use dominet_domain::mcp::McpAuditEntry;
use serde_json::{Value, json};
use std::collections::HashSet;

use crate::bridge::ActionCard;

/// What a card would do to the configuration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    /// `config_upsert` without an id.
    Create { kind: String },
    /// `config_upsert` with the id of an existing item.
    Update { kind: String, id: i64 },
    /// `config_delete`.
    Delete { kind: String, item: String },
    /// Anything else (a command, a scene, a notification, a setting): it has no snapshot.
    Other,
}

/// The card's target.
#[must_use]
pub fn target(card: &ActionCard) -> Target {
    let kind = card
        .arguments
        .get("kind")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    match card.tool.as_str() {
        "config_upsert" if !kind.is_empty() => {
            match card.arguments.get("id").and_then(Value::as_i64) {
                Some(id) => Target::Update { kind, id },
                None => Target::Create { kind },
            }
        }
        "config_delete" if !kind.is_empty() => {
            let item = match card.arguments.get("item") {
                Some(Value::String(s)) => s.clone(),
                Some(other) => other.to_string(),
                None => return Target::Other,
            };
            Target::Delete { kind, item }
        }
        _ => Target::Other,
    }
}

/// The `config_get` to make before the change (`kind`, `item`), if the change has something to take back.
#[must_use]
pub fn snapshot_ref(t: &Target) -> Option<(String, String)> {
    match t {
        Target::Update { kind, id } => Some((kind.clone(), id.to_string())),
        Target::Delete { kind, item } => Some((kind.clone(), item.clone())),
        Target::Create { .. } | Target::Other => None,
    }
}

/// What it takes to put things back, stored in the audit trail.
#[derive(Debug, Clone, PartialEq)]
pub struct UndoPlan {
    pub tool: String,
    pub arguments: Value,
    /// For a creation: the id of the new item is only known afterwards and goes in `arguments.item`.
    pub needs_created_id: bool,
}

/// Whether a change can be taken back.
#[derive(Debug, Clone, PartialEq)]
pub enum Undo {
    Plan(UndoPlan),
    /// It cannot, and why (in words for the administrator).
    Not(String),
}

/// Whether a value mentions Rhai code: its text is never readable through the tools, so it cannot be put back.
#[must_use]
pub fn involves_code(value: &Value) -> bool {
    match value {
        Value::Object(map) => map.iter().any(|(k, v)| match (k.as_str(), v) {
            ("script" | "handler_code", Value::String(s)) => !s.trim().is_empty(),
            ("has_script" | "has_handler_code", Value::Bool(true)) => true,
            ("action_type", Value::String(s)) => s == "script",
            _ => involves_code(v),
        }),
        Value::Array(items) => items.iter().any(involves_code),
        _ => false,
    }
}

/// The way back for `card`, given the item as it was (`before`, the answer of `config_get`).
#[must_use]
pub fn undo_plan(target: &Target, before: Option<&Value>, card: &ActionCard) -> Undo {
    let code = || involves_code(&card.arguments) || before.is_some_and(involves_code);
    match target {
        Target::Other => {
            Undo::Not("this action cannot be taken again (it already took place)".into())
        }
        _ if code() => {
            Undo::Not("a script's code is not readable: it cannot be put back as before".into())
        }
        Target::Create { kind } => Undo::Plan(UndoPlan {
            tool: "config_delete".into(),
            arguments: json!({"kind": kind, "item": ""}),
            needs_created_id: true,
        }),
        Target::Update { kind, id } => match before.and_then(|b| b.get("data")) {
            Some(data) => Undo::Plan(UndoPlan {
                tool: "config_upsert".into(),
                arguments: json!({"kind": kind, "id": id, "data": data}),
                needs_created_id: false,
            }),
            None => Undo::Not("the previous state could not be read".into()),
        },
        Target::Delete { kind, .. } => match before.and_then(|b| b.get("data")) {
            Some(data) => Undo::Plan(UndoPlan {
                tool: "config_upsert".into(),
                arguments: json!({"kind": kind, "data": data}),
                needs_created_id: false,
            }),
            None => Undo::Not("the previous state could not be read".into()),
        },
    }
}

impl UndoPlan {
    #[must_use]
    pub fn to_json(&self) -> Value {
        json!({
            "tool": self.tool,
            "arguments": self.arguments,
            "needs_created_id": self.needs_created_id,
        })
    }

    #[must_use]
    pub fn from_json(v: &Value) -> Option<Self> {
        Some(Self {
            tool: v.get("tool")?.as_str()?.to_string(),
            arguments: v.get("arguments")?.clone(),
            needs_created_id: v
                .get("needs_created_id")
                .and_then(Value::as_bool)
                .unwrap_or(false),
        })
    }

    /// The card that runs the undo; `created_id` fills the item of a creation's undo.
    ///
    /// # Errors
    /// The reason, when the id of the created item is needed and unknown.
    pub fn card(&self, created_id: Option<i64>) -> Result<ActionCard, String> {
        let mut arguments = self.arguments.clone();
        if self.needs_created_id {
            let id = created_id.ok_or("the created element is not identified")?;
            arguments["item"] = Value::String(id.to_string());
        }
        Ok(ActionCard {
            tool: self.tool.clone(),
            arguments,
            summary: String::new(),
            before: None,
            after: None,
        })
    }
}

/// The id of the item a `config_upsert` created, from its answer.
#[must_use]
pub fn created_id(answer: &str) -> Option<i64> {
    serde_json::from_str::<Value>(answer)
        .ok()?
        .get("details")?
        .get("id")?
        .as_i64()
}

/// A change in plain words: `Creation : scene Cinema`.
#[must_use]
pub fn describe(card: &ActionCard) -> String {
    let name = card
        .arguments
        .get("data")
        .and_then(|d| d.get("name"))
        .and_then(Value::as_str)
        .map(str::to_string)
        .or_else(|| {
            card.arguments
                .get("item")
                .and_then(Value::as_str)
                .map(str::to_string)
        });
    let kind = card
        .arguments
        .get("kind")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let what = match target(card) {
        Target::Create { .. } => "Creation",
        Target::Update { .. } => "Modification",
        Target::Delete { .. } => "Suppression",
        Target::Other => "Action",
    };
    match name {
        Some(n) if !kind.is_empty() => format!("{what} : {kind} {n}"),
        _ => format!("{what} : {}", card.tool),
    }
}

/// A confirmed change still standing, as the page lists it.
#[derive(Debug, Clone, PartialEq)]
pub struct ChangeRow {
    /// The audit line of the confirmation.
    pub id: i64,
    pub at: DateTime<Utc>,
    pub label: String,
    /// What the administrator asked, in clear.
    pub request: String,
    /// The way back, if there is one.
    pub plan: Option<UndoPlan>,
    /// Why there is none.
    pub reason: String,
    /// The id of the item a creation made.
    pub created_id: Option<i64>,
}

/// The confirmed changes that were not taken back, newest first, from the `agent` lines of the audit trail (newest first).
#[must_use]
pub fn rows(entries: &[McpAuditEntry]) -> Vec<ChangeRow> {
    let decision = |e: &McpAuditEntry| {
        e.params
            .get("decision")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string()
    };
    let undone: HashSet<i64> = entries
        .iter()
        .filter(|e| decision(e) == "undone")
        .filter_map(|e| e.params.get("of").and_then(Value::as_i64))
        .collect();
    entries
        .iter()
        .filter(|e| {
            e.kind == "agent"
                && decision(e) == "confirmed"
                && e.outcome == "ok"
                && !undone.contains(&e.id)
        })
        .map(|e| {
            let undo = e.params.get("undo");
            ChangeRow {
                id: e.id,
                at: e.at,
                label: e
                    .params
                    .get("label")
                    .and_then(Value::as_str)
                    .unwrap_or(&e.tool)
                    .to_string(),
                request: e
                    .params
                    .get("request")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
                plan: undo.and_then(UndoPlan::from_json),
                reason: undo
                    .and_then(|u| u.get("not"))
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
                created_id: serde_json::from_str::<Value>(&e.detail)
                    .ok()
                    .and_then(|d| d.get("created_id").and_then(Value::as_i64)),
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn card(tool: &str, arguments: Value) -> ActionCard {
        ActionCard {
            tool: tool.into(),
            arguments,
            summary: String::new(),
            before: None,
            after: None,
        }
    }

    #[test]
    fn a_card_is_classified_by_what_it_does_to_the_configuration() {
        let create = card(
            "config_upsert",
            json!({"kind": "scene", "data": {"name": "Cinema"}}),
        );
        let update = card(
            "config_upsert",
            json!({"kind": "scene", "id": 4, "data": {}}),
        );
        let delete = card("config_delete", json!({"kind": "scene", "item": "Cinema"}));
        let other = card("device_command", json!({"device": "x", "command": {}}));
        assert_eq!(
            target(&create),
            Target::Create {
                kind: "scene".into()
            }
        );
        assert_eq!(
            target(&update),
            Target::Update {
                kind: "scene".into(),
                id: 4
            }
        );
        assert_eq!(
            target(&delete),
            Target::Delete {
                kind: "scene".into(),
                item: "Cinema".into()
            }
        );
        assert_eq!(target(&other), Target::Other);
        assert_eq!(snapshot_ref(&target(&create)), None);
        assert_eq!(
            snapshot_ref(&target(&update)),
            Some(("scene".into(), "4".into()))
        );
    }

    #[test]
    fn each_kind_of_change_has_its_way_back() {
        let before = json!({"id": 4, "name": "Cinema", "data": {"name": "Cinema", "x": 1}});
        let update = card(
            "config_upsert",
            json!({"kind": "scene", "id": 4, "data": {"name": "Cinema", "x": 2}}),
        );
        let Undo::Plan(p) = undo_plan(&target(&update), Some(&before), &update) else {
            panic!("an update can be undone")
        };
        assert_eq!(p.tool, "config_upsert");
        assert_eq!(
            p.arguments,
            json!({"kind": "scene", "id": 4, "data": {"name": "Cinema", "x": 1}})
        );

        let delete = card("config_delete", json!({"kind": "scene", "item": "Cinema"}));
        let Undo::Plan(p) = undo_plan(&target(&delete), Some(&before), &delete) else {
            panic!("a deletion can be undone")
        };
        assert_eq!(
            p.arguments,
            json!({"kind": "scene", "data": {"name": "Cinema", "x": 1}})
        );

        let create = card(
            "config_upsert",
            json!({"kind": "scene", "data": {"name": "Neuf"}}),
        );
        let Undo::Plan(p) = undo_plan(&target(&create), None, &create) else {
            panic!("a creation can be undone")
        };
        assert!(p.needs_created_id);
        assert!(p.card(None).is_err());
        let back = p.card(Some(9)).unwrap();
        assert_eq!(
            (back.tool.as_str(), back.arguments["item"].as_str()),
            ("config_delete", Some("9"))
        );
    }

    #[test]
    fn an_action_that_already_happened_or_that_involves_code_is_not_undoable() {
        let cmd = card("device_command", json!({"device": "x"}));
        assert!(matches!(
            undo_plan(&Target::Other, None, &cmd),
            Undo::Not(_)
        ));
        let with_code = card(
            "config_upsert",
            json!({"kind": "automation", "id": 1, "data": {"name": "a", "script": "let x = 1;"}}),
        );
        let before = json!({"data": {"name": "a"}});
        assert!(matches!(
            undo_plan(&target(&with_code), Some(&before), &with_code),
            Undo::Not(_)
        ));
        let had_code = json!({"data": {"name": "a", "has_script": true}});
        let plain = card(
            "config_upsert",
            json!({"kind": "automation", "id": 1, "data": {"name": "b"}}),
        );
        assert!(matches!(
            undo_plan(&target(&plain), Some(&had_code), &plain),
            Undo::Not(_)
        ));
        // No snapshot, no undo.
        assert!(matches!(
            undo_plan(&target(&plain), None, &plain),
            Undo::Not(_)
        ));
    }

    #[test]
    fn the_plan_survives_the_audit_trail_and_the_created_id_is_read_from_the_answer() {
        let plan = UndoPlan {
            tool: "config_delete".into(),
            arguments: json!({"kind": "scene", "item": ""}),
            needs_created_id: true,
        };
        assert_eq!(UndoPlan::from_json(&plan.to_json()), Some(plan));
        assert_eq!(created_id(r#"{"done":true,"details":{"id":12}}"#), Some(12));
        assert_eq!(created_id("pas du json"), None);
    }

    #[test]
    fn the_list_keeps_confirmed_changes_that_were_not_undone() {
        let line = |id: i64, params: Value, outcome: &str, detail: &str| McpAuditEntry {
            id,
            at: Utc::now(),
            kind: "agent".into(),
            actor: "root".into(),
            actor_kind: "agent".into(),
            token_id: None,
            tool: "config_upsert".into(),
            params,
            outcome: outcome.into(),
            detail: detail.into(),
        };
        let plan = UndoPlan {
            tool: "config_delete".into(),
            arguments: json!({"kind": "scene", "item": ""}),
            needs_created_id: true,
        };
        let confirmed = |id, undo: Value| {
            line(
                id,
                json!({"decision": "confirmed", "label": "Creation : scene X", "request": "cree X", "undo": undo}),
                "ok",
                r#"{"created_id":7}"#,
            )
        };
        let entries = vec![
            line(5, json!({"decision": "undone", "of": 2}), "ok", ""),
            line(4, json!({"decision": "cancelled"}), "ok", ""),
            line(
                3,
                json!({"decision": "confirmed", "label": "boom", "undo": {}}),
                "error",
                "",
            ),
            confirmed(2, plan.to_json()),
            confirmed(1, json!({"not": "deja eu lieu"})),
        ];
        let listed = rows(&entries);
        // 5 is an undo, 4 a refusal, 3 failed, 2 was undone: only 1 stands.
        assert_eq!(listed.iter().map(|r| r.id).collect::<Vec<_>>(), [1]);
        assert_eq!(listed[0].plan, None);
        assert_eq!(listed[0].reason, "deja eu lieu");
        let both = rows(&entries[3..4]);
        assert_eq!(
            (both[0].plan.clone(), both[0].created_id),
            (Some(plan), Some(7))
        );
    }

    #[test]
    fn a_change_is_described_in_words() {
        let c = card(
            "config_upsert",
            json!({"kind": "scene", "data": {"name": "Cinema"}}),
        );
        assert_eq!(describe(&c), "Creation : scene Cinema");
        let d = card("config_delete", json!({"kind": "scene", "item": "Cinema"}));
        assert_eq!(describe(&d), "Suppression : scene Cinema");
    }
}
