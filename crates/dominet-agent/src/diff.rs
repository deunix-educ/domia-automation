// SPDX-License-Identifier: AGPL-3.0-or-later
//! What a card says it will change, in words an administrator can follow (R12 lot A4).
//!
//! Every configuration card carries a list of plain sentences (« nom : Cinema », « description : RDC -> etage »). For an
//! **automation** the card also shows the item as YAML before and after, line by line, because an automation is the one thing
//! whose shape (trigger, action) cannot be told in a sentence. Nothing here decides anything: it only describes the card.

use serde_json::Value;

use crate::bridge::ActionCard;
use crate::changes::{Target, target};

/// A line of the before / after comparison.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mark {
    Same,
    Added,
    Removed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiffLine {
    pub mark: Mark,
    pub text: String,
}

impl DiffLine {
    /// `+ text`, `- text` or two spaces and the text: the form the page receives.
    #[must_use]
    pub fn render(&self) -> String {
        let sign = match self.mark {
            Mark::Same => ' ',
            Mark::Added => '+',
            Mark::Removed => '-',
        };
        format!("{sign} {}", self.text)
    }
}

/// What the page shows under a card.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CardView {
    /// Plain sentences, one per field.
    pub lines: Vec<String>,
    /// The YAML comparison (automations only), empty otherwise.
    pub diff: Vec<DiffLine>,
}

/// Longest YAML (in lines) compared line by line; beyond it the comparison is not shown (the sentences remain).
const MAX_DIFF_LINES: usize = 300;

/// The view of a card: nothing for what is not a change to the configuration.
#[must_use]
pub fn view(card: &ActionCard) -> CardView {
    // The server's own plan when there is one (defaults filled in, so that a comparison shows real changes), else what was written.
    let after = card.after.as_ref().or_else(|| card.arguments.get("data"));
    let before = card.before.as_ref();
    let kind = card
        .arguments
        .get("kind")
        .and_then(Value::as_str)
        .unwrap_or_default();
    match target(card) {
        Target::Create { .. } => CardView {
            lines: after.map(|a| fields(a, None)).unwrap_or_default(),
            diff: yaml_diff(kind, None, after),
        },
        Target::Update { .. } => CardView {
            lines: after.map(|a| fields(a, before)).unwrap_or_default(),
            diff: yaml_diff(kind, before, after),
        },
        Target::Delete { .. } => CardView {
            lines: before
                .map(|b| {
                    let mut l = vec!["Sera supprime :".to_string()];
                    l.extend(fields(b, None));
                    l
                })
                .unwrap_or_default(),
            diff: yaml_diff(kind, before, None),
        },
        Target::Other => CardView {
            lines: sentence(card).into_iter().collect(),
            diff: Vec::new(),
        },
    }
}

/// One plain sentence for the actions a household understands (a command, a scene, an acknowledgement, a banner); the others
/// are shown by their tool and arguments.
fn sentence(card: &ActionCard) -> Option<String> {
    let arg = |k: &str| card.arguments.get(k);
    let text = |k: &str| arg(k).and_then(Value::as_str).map(str::to_string);
    match card.tool.as_str() {
        "device_command" => {
            let device = text("device")?;
            let command = arg("command")?.as_object()?;
            let what: Vec<String> = command.iter().map(|(k, v)| command_words(k, v)).collect();
            Some(format!("{} : {device}", what.join(", ")))
        }
        "scene_run" => Some(format!("Lancer la scene : {}", text("scene")?)),
        "alert_ack" => Some(if arg("all").and_then(Value::as_bool) == Some(true) {
            "Acquitter toutes les alertes en cours".to_string()
        } else {
            format!(
                "Acquitter l'alerte {}",
                arg("alert").map(short_value).unwrap_or_default()
            )
        }),
        "notify_broadcast" => Some(format!("Afficher ce message a tous : {}", text("text")?)),
        _ => None,
    }
}

/// A command field in words: `state = CLOSED` becomes "Fermer".
fn command_words(key: &str, value: &Value) -> String {
    let upper = value.as_str().map(str::to_uppercase);
    match (key, upper.as_deref()) {
        ("state", Some("ON")) => "Allumer".to_string(),
        ("state", Some("OFF")) => "Eteindre".to_string(),
        ("state", Some("TOGGLE")) => "Inverser".to_string(),
        ("state", Some("OPEN")) => "Ouvrir".to_string(),
        ("state", Some("CLOSE" | "CLOSED")) => "Fermer".to_string(),
        ("state", Some("STOP")) => "Arreter".to_string(),
        ("position", _) => format!("Mettre a {} %", short_value(value)),
        ("brightness", _) => format!("Regler la luminosite a {}", short_value(value)),
        _ => format!("Regler {} a {}", field_name(key), short_value(value)),
    }
}

fn yaml_diff(kind: &str, before: Option<&Value>, after: Option<&Value>) -> Vec<DiffLine> {
    if kind != "automation" {
        return Vec::new();
    }
    let text = |v: Option<&Value>| v.map(to_yaml).unwrap_or_default();
    line_diff(&text(before), &text(after))
}

/// The top-level fields in words. With `before`, only what changes (`old -> new`); without, everything that is set.
fn fields(after: &Value, before: Option<&Value>) -> Vec<String> {
    let Some(map) = after.as_object() else {
        return Vec::new();
    };
    let mut keys: Vec<&String> = map.keys().collect();
    keys.sort();
    let mut out = Vec::new();
    for key in keys {
        let new = &map[key];
        let name = field_name(key);
        match before.and_then(|b| b.get(key)) {
            Some(old) if old == new => {}
            Some(old) => out.push(format!(
                "{name} : {} -> {}",
                short_value(old),
                short_value(new)
            )),
            None if before.is_some() && new.is_null() => {}
            None if new.is_null() => {}
            None => out.push(format!("{name} : {}", short_value(new))),
        }
    }
    if out.is_empty() && before.is_some() {
        out.push("aucun champ ne change".to_string());
    }
    out
}

/// The name of a field as the house would say it (the ones everybody meets), otherwise the name as it is.
fn field_name(key: &str) -> &str {
    match key {
        "name" => "nom",
        "description" => "description",
        "enabled" => "actif",
        "trigger_type" => "declencheur",
        "trigger_config" => "reglage du declencheur",
        "action_type" => "action",
        "action_config" => "reglage de l'action",
        "condition" => "condition",
        "title" => "titre",
        "icon" => "icone",
        "position" => "place",
        "level" => "niveau",
        other => other,
    }
}

/// A value on one line: texts as they are, structures as compact JSON, cut at 80 characters.
fn short_value(v: &Value) -> String {
    let text = match v {
        Value::String(s) if s.is_empty() => "(vide)".to_string(),
        Value::String(s) => s.clone(),
        Value::Null => "(rien)".to_string(),
        other => other.to_string(),
    };
    if text.chars().count() > 80 {
        let mut cut: String = text.chars().take(80).collect();
        cut.push_str("...");
        cut
    } else {
        text
    }
}

/// The value as YAML, keys in alphabetical order (the same item always gives the same text, so a comparison shows real changes).
#[must_use]
pub fn to_yaml(value: &Value) -> String {
    let mut out = String::new();
    write_node(value, 0, &mut out);
    out
}

fn scalar(v: &Value) -> String {
    match v {
        Value::Null => "null".into(),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => n.to_string(),
        Value::String(s) => {
            let plain = !s.is_empty()
                && s.chars()
                    .all(|c| c.is_alphanumeric() || " _./+-".contains(c))
                && !s.starts_with(['-', ' '])
                && !s.ends_with(' ')
                && s.parse::<f64>().is_err()
                && !matches!(
                    s.as_str(),
                    "true" | "false" | "null" | "yes" | "no" | "on" | "off"
                );
            if plain {
                s.clone()
            } else {
                serde_json::to_string(s).unwrap_or_default()
            }
        }
        Value::Array(_) | Value::Object(_) => String::new(),
    }
}

fn is_scalar(v: &Value) -> bool {
    !matches!(v, Value::Array(_) | Value::Object(_))
}

fn write_node(value: &Value, indent: usize, out: &mut String) {
    let pad = " ".repeat(indent);
    match value {
        Value::Object(map) if map.is_empty() => out.push_str(&format!("{pad}{{}}\n")),
        Value::Array(items) if items.is_empty() => out.push_str(&format!("{pad}[]\n")),
        Value::Object(map) => {
            let mut entries: Vec<(&String, &Value)> = map.iter().collect();
            entries.sort_by(|a, b| a.0.cmp(b.0));
            for (k, v) in entries {
                if is_scalar(v) {
                    out.push_str(&format!("{pad}{k}: {}\n", scalar(v)));
                } else if matches!(v, Value::Object(m) if m.is_empty()) {
                    out.push_str(&format!("{pad}{k}: {{}}\n"));
                } else if matches!(v, Value::Array(a) if a.is_empty()) {
                    out.push_str(&format!("{pad}{k}: []\n"));
                } else {
                    out.push_str(&format!("{pad}{k}:\n"));
                    write_node(v, indent + 2, out);
                }
            }
        }
        Value::Array(items) => {
            for item in items {
                if is_scalar(item) {
                    out.push_str(&format!("{pad}- {}\n", scalar(item)));
                } else {
                    out.push_str(&format!("{pad}-\n"));
                    write_node(item, indent + 2, out);
                }
            }
        }
        other => out.push_str(&format!("{pad}{}\n", scalar(other))),
    }
}

/// Line by line comparison of two texts (longest common subsequence). Empty if the texts are too long to compare here.
#[must_use]
pub fn line_diff(before: &str, after: &str) -> Vec<DiffLine> {
    let a: Vec<&str> = before.lines().collect();
    let b: Vec<&str> = after.lines().collect();
    if a.len() > MAX_DIFF_LINES || b.len() > MAX_DIFF_LINES {
        return Vec::new();
    }
    // lcs[i][j]: length of the common subsequence of a[i..] and b[j..].
    let mut lcs = vec![vec![0u16; b.len() + 1]; a.len() + 1];
    for i in (0..a.len()).rev() {
        for j in (0..b.len()).rev() {
            lcs[i][j] = if a[i] == b[j] {
                lcs[i + 1][j + 1] + 1
            } else {
                lcs[i + 1][j].max(lcs[i][j + 1])
            };
        }
    }
    let (mut i, mut j) = (0, 0);
    let mut out = Vec::new();
    while i < a.len() || j < b.len() {
        if i < a.len() && j < b.len() && a[i] == b[j] {
            out.push(DiffLine {
                mark: Mark::Same,
                text: a[i].to_string(),
            });
            i += 1;
            j += 1;
        } else if i < a.len() && (j == b.len() || lcs[i + 1][j] >= lcs[i][j + 1]) {
            // A removal is shown before the addition that replaces it.
            out.push(DiffLine {
                mark: Mark::Removed,
                text: a[i].to_string(),
            });
            i += 1;
        } else {
            out.push(DiffLine {
                mark: Mark::Added,
                text: b[j].to_string(),
            });
            j += 1;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn a_device_command_is_said_in_plain_words() {
        let say = |args: Value| view(&card("device_command", args, None)).lines;
        assert_eq!(
            say(json!({"device": "Volet-salon", "command": {"state": "CLOSED"}})),
            vec!["Fermer : Volet-salon".to_string()]
        );
        assert_eq!(
            say(json!({"device": "Lampe", "command": {"state": "off"}})),
            vec!["Eteindre : Lampe".to_string()]
        );
        assert_eq!(
            say(json!({"device": "Volet", "command": {"position": 40}})),
            vec!["Mettre a 40 % : Volet".to_string()]
        );
        let scene = view(&card("scene_run", json!({"scene": "Cinema"}), None)).lines;
        assert_eq!(scene, vec!["Lancer la scene : Cinema".to_string()]);
        // A tool with no sentence keeps its tool and arguments on the card.
        assert!(
            view(&card("camera_ptz", json!({"camera": "Jardin"}), None))
                .lines
                .is_empty()
        );
    }

    fn card(tool: &str, arguments: Value, before: Option<Value>) -> ActionCard {
        ActionCard {
            tool: tool.into(),
            arguments,
            summary: String::new(),
            before,
            after: None,
        }
    }

    #[test]
    fn yaml_is_stable_and_quotes_what_could_be_misread() {
        let v = json!({"b": 1, "a": {"x": ["un", "deux"], "empty": {}}, "t": "true", "n": "12", "s": "a: b", "ok": "volet salon"});
        let y = to_yaml(&v);
        assert_eq!(
            y,
            "a:\n  empty: {}\n  x:\n    - un\n    - deux\nb: 1\nn: \"12\"\nok: volet salon\ns: \"a: b\"\nt: \"true\"\n"
        );
        // Key order of the input does not matter.
        assert_eq!(
            to_yaml(&json!({"z": 1, "a": 2})),
            to_yaml(&json!({"a": 2, "z": 1}))
        );
    }

    #[test]
    fn the_line_diff_shows_what_changed_and_nothing_else() {
        let d = line_diff("a: 1\nb: 2\nc: 3\n", "a: 1\nb: 5\nc: 3\nd: 4\n");
        let shown: Vec<String> = d.iter().map(DiffLine::render).collect();
        assert_eq!(shown, ["  a: 1", "- b: 2", "+ b: 5", "  c: 3", "+ d: 4"]);
        assert!(line_diff("x\n", "x\n").iter().all(|l| l.mark == Mark::Same));
    }

    #[test]
    fn a_new_automation_shows_its_fields_and_a_yaml_made_only_of_additions() {
        let data = json!({"name": "Volets au coucher du soleil", "enabled": true, "trigger_type": "solar", "trigger_config": {"event": "sunset"}});
        let v = view(&card(
            "config_upsert",
            json!({"kind": "automation", "data": data}),
            None,
        ));
        assert!(
            v.lines
                .contains(&"nom : Volets au coucher du soleil".to_string()),
            "{:?}",
            v.lines
        );
        assert!(v.lines.iter().any(|l| l.starts_with("declencheur : solar")));
        assert!(!v.diff.is_empty() && v.diff.iter().all(|l| l.mark == Mark::Added));
    }

    #[test]
    fn an_update_lists_only_the_fields_that_change() {
        let before = json!({"name": "Salon", "description": "RDC"});
        let c = card(
            "config_upsert",
            json!({"kind": "location", "id": 3, "data": {"name": "Salon", "description": "etage"}}),
            Some(before),
        );
        let v = view(&c);
        assert_eq!(v.lines, ["description : RDC -> etage"]);
        // The YAML comparison is for automations only.
        assert!(v.diff.is_empty());
    }

    #[test]
    fn a_deletion_says_what_goes_and_what_is_not_a_change_has_no_view() {
        let c = card(
            "config_delete",
            json!({"kind": "location", "item": "Salon"}),
            Some(json!({"name": "Salon", "description": "RDC"})),
        );
        let v = view(&c);
        assert_eq!(v.lines[0], "Sera supprime :");
        assert!(v.lines.contains(&"nom : Salon".to_string()));
        assert_eq!(
            view(&card("device_command", json!({"device": "x"}), None)),
            CardView::default()
        );
    }
}
