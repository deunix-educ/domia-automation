// SPDX-License-Identifier: AGPL-3.0-or-later
//! The assistant's window on the application: an MCP client talking in memory to the real tool server, as the signed-in account.
//! Same tools, same scopes, same audit trail as over HTTP, no token, no network.
//!
//! The rule of R12 lives here: **the model never changes anything**. A tool outside the `lecture` scope is only ever called with
//! `dry_run = true`; its answer becomes an [`ActionCard`]. `confirm` and `dry_run` are removed from the schemas the model sees.

use dominet_domain::mcp::{Actor, Scope};
use dominet_mcp::{DominetMcp, scope_of};
use dominet_server::AppState;
use rmcp::model::{CallToolRequestParams, ClientConfig};
use rmcp::service::RunningService;
use rmcp::{RoleClient, RoleServer, ServiceExt};
use serde_json::{Map, Value};

use crate::llm::ToolDef;
use crate::profile::Regime;

/// Arguments the model never sees nor writes: the agent decides them.
const HIDDEN: [&str; 2] = ["confirm", "dry_run"];

#[derive(Debug, thiserror::Error)]
pub enum BridgeError {
    #[error("liaison avec les outils impossible : {0}")]
    Connect(String),
    /// The tool is not offered in this regime (or does not exist): the model made it up, or asks for more than it may.
    #[error("l'outil {0} n'existe pas ou n'est pas disponible")]
    NotOffered(String),
    #[error("appel de l'outil impossible : {0}")]
    Call(String),
}

/// What a tool call gave back.
#[derive(Debug, Clone, PartialEq)]
pub enum Outcome {
    /// A read (or a refusal explained in words): data for the model.
    Data(String),
    /// An action to confirm: nothing has been done.
    Card(Box<ActionCard>),
}

/// An action proposed by the model, waiting for the user's click.
#[derive(Debug, Clone, PartialEq)]
pub struct ActionCard {
    pub tool: String,
    /// The arguments as the model wrote them (without `dry_run` and `confirm`).
    pub arguments: Value,
    /// What the server says it would do (the `dry_run` answer, as text).
    pub summary: String,
    /// For a change to an existing item of the configuration: the item as it is now (the `data` of `config_get`), to show what
    /// changes.
    pub before: Option<Value>,
    /// The item as it would be once written, as the server's own plan normalises it (defaults filled in), for `config_upsert`.
    pub after: Option<Value>,
}

struct Offered {
    def: ToolDef,
    scope: Scope,
    /// The tool asks for `confirm = true` (deletions); it is only ever sent by [`Bridge::confirm`].
    wants_confirm: bool,
}

/// The in-memory link to the tool server.
pub struct Bridge {
    regime: Regime,
    client: RunningService<RoleClient, ClientConfig>,
    // Kept alive for the duration of the link.
    _server: RunningService<RoleServer, DominetMcp>,
    offered: Vec<Offered>,
}

impl Bridge {
    /// Starts the tool server for `actor` and lists the tools `regime` offers it.
    ///
    /// # Errors
    /// [`BridgeError::Connect`] if the in-memory link cannot be set up.
    pub async fn connect(
        state: AppState,
        actor: Actor,
        regime: Regime,
    ) -> Result<Self, BridgeError> {
        let (server_io, client_io) = tokio::io::duplex(1 << 20);
        let server = DominetMcp::with_actor(state, actor);
        let (server, client) = tokio::join!(
            server.serve(server_io),
            ClientConfig::default().serve(client_io)
        );
        let server = server.map_err(|e| BridgeError::Connect(e.to_string()))?;
        let client = client.map_err(|e| BridgeError::Connect(e.to_string()))?;
        let listed = client
            .list_all_tools()
            .await
            .map_err(|e| BridgeError::Connect(e.to_string()))?;
        let offered = listed
            .into_iter()
            .filter_map(|t| {
                let scope = scope_of(&t.name)?;
                regime.offers(&t.name, scope).then(|| Offered {
                    wants_confirm: t
                        .input_schema
                        .get("properties")
                        .is_some_and(|p| p.get("confirm").is_some()),
                    def: tool_def(&t),
                    scope,
                })
            })
            .collect();
        Ok(Self {
            regime,
            client,
            _server: server,
            offered,
        })
    }

    /// The regime this link was opened for.
    #[must_use]
    pub fn regime(&self) -> Regime {
        self.regime
    }

    /// The tools shown to the model.
    #[must_use]
    pub fn tools(&self) -> Vec<ToolDef> {
        self.offered.iter().map(|o| o.def.clone()).collect()
    }

    /// Calls a tool for the model. A read runs; any other tool is only simulated (`dry_run`) and gives a card.
    ///
    /// # Errors
    /// [`BridgeError::NotOffered`] for a tool outside the regime; [`BridgeError::Call`] if the link breaks.
    pub async fn call(&self, name: &str, arguments: &Value) -> Result<Outcome, BridgeError> {
        let offered = self
            .offered
            .iter()
            .find(|o| o.def.name == name)
            .ok_or_else(|| BridgeError::NotOffered(name.to_string()))?;
        let mut args: Map<String, Value> = arguments.as_object().cloned().unwrap_or_default();
        for hidden in HIDDEN {
            args.remove(hidden);
        }
        repair(&mut args, &offered.def.parameters);
        let action = offered.scope != Scope::Read;
        let shown = Value::Object(args.clone());
        if action {
            args.insert("dry_run".into(), Value::Bool(true));
        }
        let result = self
            .client
            .call_tool(CallToolRequestParams::new(name.to_string()).with_arguments(args))
            .await
            .map_err(|e| BridgeError::Call(e.to_string()))?;
        let mut text = content_text(&result.content);
        let mut sentence = None;
        let mut planned: (Option<Value>, Option<Value>) = (None, None);
        if let Some(structured) = &result.structured_content {
            let details = structured.get("details");
            let pick = |key: &str| {
                details
                    .and_then(|d| d.get(key))
                    .filter(|v| !v.is_null())
                    .cloned()
            };
            planned = (pick("before"), pick("after"));
            // The structured answer usually repeats the text block: it is added only when it says something more.
            let same = serde_json::from_str::<Value>(&text).is_ok_and(|t| &t == structured);
            if !same {
                if !text.is_empty() {
                    text.push('\n');
                }
                text.push_str(&structured.to_string());
            }
            sentence = structured
                .get("summary")
                .and_then(Value::as_str)
                .map(str::to_string);
        }
        if action && result.is_error != Some(true) {
            let mut card = ActionCard {
                tool: name.to_string(),
                arguments: shown,
                summary: sentence.unwrap_or(text),
                before: planned.0,
                after: planned.1,
            };
            if card.before.is_none() {
                card.before = self.current_item(&card).await;
            }
            Ok(Outcome::Card(Box::new(card)))
        } else {
            Ok(Outcome::Data(text))
        }
    }
}

impl Bridge {
    /// The item a configuration card changes or deletes, as it is now (nothing for a creation or another kind of card).
    async fn current_item(&self, card: &ActionCard) -> Option<Value> {
        let (kind, item) = crate::changes::snapshot_ref(&crate::changes::target(card))?;
        let result = self
            .client
            .call_tool(
                CallToolRequestParams::new("config_get").with_arguments(
                    serde_json::json!({"kind": kind, "item": item})
                        .as_object()
                        .cloned()
                        .unwrap_or_default(),
                ),
            )
            .await
            .ok()?;
        if result.is_error == Some(true) {
            return None;
        }
        result
            .structured_content
            .as_ref()
            .and_then(|v| v.get("data").cloned())
    }

    /// Runs a card **for real**, because the user clicked "Confirm". This is the only way a change is made, and the model has no
    /// access to it: [`Bridge::call`] never sends anything but `dry_run = true` for an action.
    ///
    /// Returns the tool's answer as text.
    ///
    /// # Errors
    /// [`BridgeError::NotOffered`] if the tool is outside the regime (or is a read); [`BridgeError::Call`] if the link breaks or
    /// the tool refuses.
    pub async fn confirm(&self, card: &ActionCard) -> Result<String, BridgeError> {
        let offered = self
            .offered
            .iter()
            .find(|o| o.def.name == card.tool && o.scope != Scope::Read)
            .ok_or_else(|| BridgeError::NotOffered(card.tool.clone()))?;
        let mut args: Map<String, Value> = card.arguments.as_object().cloned().unwrap_or_default();
        for hidden in HIDDEN {
            args.remove(hidden);
        }
        if offered.wants_confirm {
            args.insert("confirm".into(), Value::Bool(true));
        }
        let result = self
            .client
            .call_tool(CallToolRequestParams::new(card.tool.clone()).with_arguments(args))
            .await
            .map_err(|e| BridgeError::Call(e.to_string()))?;
        let text = content_text(&result.content);
        if result.is_error == Some(true) {
            return Err(BridgeError::Call(text));
        }
        Ok(text)
    }
}

/// The text blocks of a result (the other kinds, images, are not shown to the model).
fn content_text(content: &impl serde::Serialize) -> String {
    serde_json::to_value(content)
        .ok()
        .and_then(|v| v.as_array().cloned())
        .unwrap_or_default()
        .iter()
        .filter_map(|c| c.get("text").and_then(Value::as_str))
        .collect::<Vec<_>>()
        .join("\n")
}

/// A tool as the model sees it: the arguments the agent decides are removed, and the schema is simplified (JSON Schema keywords
/// that model servers turn into grammars badly: `$schema`, `format`, nullable type unions).
fn tool_def(tool: &rmcp::model::Tool) -> ToolDef {
    let schema = serde_json::to_value(&*tool.input_schema).unwrap_or(Value::Null);
    let mut parameters = simplify(schema);
    if let Some(props) = parameters
        .get_mut("properties")
        .and_then(Value::as_object_mut)
    {
        for hidden in HIDDEN {
            props.remove(hidden);
        }
    }
    if let Some(required) = parameters.get_mut("required").and_then(Value::as_array_mut) {
        required.retain(|r| !HIDDEN.iter().any(|h| r == h));
    }
    ToolDef {
        name: tool.name.to_string(),
        description: tool.description.as_deref().unwrap_or_default().to_string(),
        parameters,
    }
}

/// Small models sometimes write an object argument as a JSON *string* (`"command": "{\"state\": \"OPEN\"}"`), above all when the
/// schema does not say what the value is. A string that is valid JSON for an object or an array is parsed back, unless the schema
/// declares the argument as a string.
fn repair(args: &mut Map<String, Value>, parameters: &Value) {
    for (key, value) in args.iter_mut() {
        let Value::String(text) = value else { continue };
        let declared = parameters
            .get("properties")
            .and_then(|p| p.get(key))
            .and_then(|p| p.get("type"))
            .and_then(Value::as_str);
        if declared == Some("string") {
            continue;
        }
        if let Ok(parsed @ (Value::Object(_) | Value::Array(_))) = serde_json::from_str(text.trim())
        {
            *value = parsed;
        }
    }
}

fn simplify(value: Value) -> Value {
    match value {
        Value::Object(map) => Value::Object(
            map.into_iter()
                .filter(|(k, _)| k != "$schema" && k != "format")
                .map(|(k, v)| {
                    if let (true, Value::Array(types)) = (k == "type", &v) {
                        let first = types
                            .iter()
                            .find(|t| *t != "null")
                            .cloned()
                            .unwrap_or_else(|| Value::String("string".into()));
                        return (k, first);
                    }
                    (k, simplify(v))
                })
                .collect(),
        ),
        Value::Array(items) => Value::Array(items.into_iter().map(simplify).collect()),
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::repair;

    use super::simplify;

    #[test]
    fn an_object_written_as_a_string_is_parsed_back_unless_the_schema_says_string() {
        let schema = json!({"properties": {"command": {}, "name": {"type": "string"}, "n": {"type": "integer"}}});
        let mut args = json!({"command": "{\"state\": \"OPEN\"}", "name": "{\"a\": 1}", "n": 3})
            .as_object()
            .cloned()
            .unwrap();
        repair(&mut args, &schema);
        assert_eq!(args["command"], json!({"state": "OPEN"}));
        assert_eq!(args["name"], json!("{\"a\": 1}"));
        assert_eq!(args["n"], json!(3));
    }

    #[test]
    fn unions_with_null_and_formats_are_simplified() {
        let schema = json!({"$schema": "x", "properties": {
            "n": {"type": ["integer", "null"], "format": "int64"},
            "t": {"type": "string"}}});
        assert_eq!(
            simplify(schema),
            json!({"properties": {"n": {"type": "integer"}, "t": {"type": "string"}}})
        );
    }
}
