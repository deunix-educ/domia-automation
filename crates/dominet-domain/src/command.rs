// SPDX-License-Identifier: AGPL-3.0-or-later
//! Commandes envoyees aux appareils (R3 lot 5, ADR 0029) : ce que le serveur accepte de publier sur `<appareil>/set`.
//!
//! L'implementation de reference publiait n'importe quel JSON. Ici la charge doit etre un objet non vide, de profondeur et de taille bornees :
//! l'interface n'envoie que des objets plats (`{"state":"ON"}`, `{"brightness":128}`), Zigbee2MQTT accepte quelques objets imbriques
//! (`{"color":{"x":0.3,"y":0.4}}`), rien d'autre n'a de sens.

use serde_json::Value;

/// Taille maximale du JSON publie, en octets.
pub const MAX_COMMAND_BYTES: usize = 1024;
/// Profondeur maximale d'imbrication (l'objet racine compte pour 1).
pub const MAX_COMMAND_DEPTH: usize = 3;

/// Pourquoi une commande est refusee.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandProblem {
    NotJson,
    NotAnObject,
    Empty,
    TooLarge,
    TooDeep,
}

fn depth(v: &Value) -> usize {
    match v {
        Value::Object(o) => 1 + o.values().map(depth).max().unwrap_or(0),
        Value::Array(a) => 1 + a.iter().map(depth).max().unwrap_or(0),
        _ => 0,
    }
}

/// Analyse le texte d'une commande.
///
/// # Errors
/// [`CommandProblem`] si ce n'est pas un objet JSON non vide, trop gros ou trop imbrique.
pub fn parse_command(text: &str) -> Result<Value, CommandProblem> {
    if text.len() > MAX_COMMAND_BYTES {
        return Err(CommandProblem::TooLarge);
    }
    let value: Value = serde_json::from_str(text).map_err(|_| CommandProblem::NotJson)?;
    check_command(&value)?;
    Ok(value)
}

/// Verifie une commande deja analysee (scenes, automations) : objet non vide, 3 niveaux au plus, 1 Ko au plus une fois ecrit.
///
/// # Errors
/// [`CommandProblem`].
pub fn check_command(value: &Value) -> Result<(), CommandProblem> {
    match value {
        Value::Object(o) if o.is_empty() => Err(CommandProblem::Empty),
        Value::Object(_) if depth(value) > MAX_COMMAND_DEPTH => Err(CommandProblem::TooDeep),
        Value::Object(_) if value.to_string().len() > MAX_COMMAND_BYTES => {
            Err(CommandProblem::TooLarge)
        }
        Value::Object(_) => Ok(()),
        _ => Err(CommandProblem::NotAnObject),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flat_and_small_nested_objects_are_accepted() {
        assert!(parse_command(r#"{"state":"ON"}"#).is_ok());
        assert!(parse_command(r#"{"brightness":128,"transition":1.5}"#).is_ok());
        assert!(parse_command(r#"{"color":{"x":0.3,"y":0.4}}"#).is_ok());
    }

    #[test]
    fn anything_else_is_refused() {
        assert_eq!(parse_command("pas du json"), Err(CommandProblem::NotJson));
        assert_eq!(parse_command(""), Err(CommandProblem::NotJson));
        for t in ["[1]", "3", "\"ON\"", "null", "true"] {
            assert_eq!(parse_command(t), Err(CommandProblem::NotAnObject), "{t}");
        }
        assert_eq!(parse_command("{}"), Err(CommandProblem::Empty));
        assert_eq!(
            parse_command(r#"{"a":{"b":{"c":{"d":1}}}}"#),
            Err(CommandProblem::TooDeep)
        );
        let big = format!(r#"{{"a":"{}"}}"#, "x".repeat(MAX_COMMAND_BYTES));
        assert_eq!(parse_command(&big), Err(CommandProblem::TooLarge));
    }
}
