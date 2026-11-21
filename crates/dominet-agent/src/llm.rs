// SPDX-License-Identifier: AGPL-3.0-or-later
//! The model behind the agent: a trait, the `llama-server` implementation (OpenAI-compatible `/v1/chat/completions` with `tools`)
//! and a scripted fake, so that the agent is tested without a GPU or a model.
//!
//! Constrained decoding: `llama-server` started with `--jinja` builds the grammar of the tool calls from the `tools` it receives;
//! nothing more is needed here (the A0 bench read no invalid arguments in 420 runs, 14 model and mode pairs).

use std::collections::VecDeque;
use std::sync::Mutex;
use std::time::Duration;

use async_trait::async_trait;
use serde_json::{Value, json};

/// A tool call asked by the model.
#[derive(Debug, Clone, PartialEq)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    /// The arguments as a JSON value; a model that wrote invalid JSON gives a `Value::String` holding the raw text.
    pub arguments: Value,
}

/// A message of the conversation.
#[derive(Debug, Clone, PartialEq)]
pub enum Message {
    System(String),
    User(String),
    Assistant {
        content: String,
        tool_calls: Vec<ToolCall>,
    },
    Tool {
        call_id: String,
        content: String,
    },
}

/// A tool shown to the model.
#[derive(Debug, Clone, PartialEq)]
pub struct ToolDef {
    pub name: String,
    pub description: String,
    /// JSON Schema of the arguments.
    pub parameters: Value,
}

/// What the model answered: some text, or tool calls (or both).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Reply {
    pub content: String,
    pub tool_calls: Vec<ToolCall>,
}

#[derive(Debug, thiserror::Error)]
pub enum LlmError {
    /// The model server does not answer (switched off machine, network).
    #[error("modele injoignable : {0}")]
    Unreachable(String),
    /// The model server answered with an error.
    #[error("le serveur de modele a repondu {status} : {body}")]
    Status { status: u16, body: String },
    /// The answer is not what the API promises.
    #[error("reponse du modele illisible : {0}")]
    Invalid(String),
}

/// A chat model that may call tools.
#[async_trait]
pub trait LlmBackend: Send + Sync {
    /// One completion for the conversation, with the tools it may call.
    ///
    /// # Errors
    /// [`LlmError`] when the model server is unreachable or answers badly.
    async fn chat(&self, messages: &[Message], tools: &[ToolDef]) -> Result<Reply, LlmError>;
}

/// `llama-server` (llama.cpp) or any OpenAI-compatible server.
pub struct OpenAiCompat {
    http: reqwest::Client,
    url: String,
    model: String,
}

impl OpenAiCompat {
    /// `base_url` is the server (`http://host:8080`), `model` the name sent in the request.
    ///
    /// # Panics
    /// If the HTTP client cannot be built (no TLS backend), which does not happen with the workspace features.
    #[must_use]
    pub fn new(base_url: &str, model: &str, timeout: Duration) -> Self {
        Self {
            http: reqwest::Client::builder()
                .timeout(timeout)
                .build()
                .expect("client HTTP"),
            url: format!("{}/v1/chat/completions", base_url.trim_end_matches('/')),
            model: model.to_string(),
        }
    }

    /// The request body: temperature 0, thinking off (Qwen3 templates), prefix cache on.
    #[must_use]
    pub fn body(&self, messages: &[Message], tools: &[ToolDef]) -> Value {
        let mut body = json!({
            "model": self.model,
            "messages": messages.iter().map(wire_message).collect::<Vec<_>>(),
            "temperature": 0,
            "max_tokens": 512,
            "chat_template_kwargs": {"enable_thinking": false},
            "cache_prompt": true,
        });
        if !tools.is_empty() {
            body["tools"] = tools
                .iter()
                .map(|t| {
                    json!({"type": "function", "function": {
                        "name": t.name, "description": t.description, "parameters": t.parameters}})
                })
                .collect();
        }
        body
    }
}

fn wire_message(m: &Message) -> Value {
    match m {
        Message::System(c) => json!({"role": "system", "content": c}),
        Message::User(c) => json!({"role": "user", "content": c}),
        Message::Assistant {
            content,
            tool_calls,
        } => {
            let mut v = json!({"role": "assistant", "content": content});
            if !tool_calls.is_empty() {
                v["tool_calls"] = tool_calls
                    .iter()
                    .map(|c| {
                        json!({"id": c.id, "type": "function", "function": {
                            "name": c.name, "arguments": c.arguments.to_string()}})
                    })
                    .collect();
            }
            v
        }
        Message::Tool { call_id, content } => {
            json!({"role": "tool", "tool_call_id": call_id, "content": content})
        }
    }
}

/// Reads the first choice of a chat completion.
///
/// # Errors
/// [`LlmError::Invalid`] when the body has no message.
pub fn parse_reply(body: &Value) -> Result<Reply, LlmError> {
    let message = body
        .pointer("/choices/0/message")
        .ok_or_else(|| LlmError::Invalid("no choices[0].message".into()))?;
    let content = message
        .get("content")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let mut tool_calls = Vec::new();
    for (i, call) in message
        .get("tool_calls")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .enumerate()
    {
        let function = call
            .get("function")
            .ok_or_else(|| LlmError::Invalid("tool call without a function".into()))?;
        let name = function
            .get("name")
            .and_then(Value::as_str)
            .ok_or_else(|| LlmError::Invalid("tool call without a name".into()))?
            .to_string();
        // Servers send the arguments as a JSON string; a few send an object.
        let arguments = match function.get("arguments") {
            Some(Value::String(raw)) if raw.trim().is_empty() => json!({}),
            Some(Value::String(raw)) => {
                serde_json::from_str(raw).unwrap_or_else(|_| Value::String(raw.clone()))
            }
            Some(other) => other.clone(),
            None => json!({}),
        };
        let id = call
            .get("id")
            .and_then(Value::as_str)
            .map_or_else(|| format!("call_{i}"), ToString::to_string);
        tool_calls.push(ToolCall {
            id,
            name,
            arguments,
        });
    }
    Ok(Reply {
        content,
        tool_calls,
    })
}

#[async_trait]
impl LlmBackend for OpenAiCompat {
    async fn chat(&self, messages: &[Message], tools: &[ToolDef]) -> Result<Reply, LlmError> {
        let response = self
            .http
            .post(&self.url)
            .json(&self.body(messages, tools))
            .send()
            .await
            .map_err(|e| LlmError::Unreachable(e.to_string()))?;
        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            return Err(LlmError::Status {
                status: status.as_u16(),
                body: body.chars().take(300).collect(),
            });
        }
        let body: Value = response
            .json()
            .await
            .map_err(|e| LlmError::Invalid(e.to_string()))?;
        parse_reply(&body)
    }
}

/// One call received by a [`Scripted`] model.
#[derive(Debug, Clone)]
pub struct Seen {
    pub messages: Vec<Message>,
    pub tools: Vec<ToolDef>,
}

/// A fake model that plays prepared replies in order and remembers what it was sent.
#[derive(Default)]
pub struct Scripted {
    replies: Mutex<VecDeque<Result<Reply, String>>>,
    seen: Mutex<Vec<Seen>>,
}

impl Scripted {
    #[must_use]
    pub fn new(replies: impl IntoIterator<Item = Reply>) -> Self {
        Self {
            replies: Mutex::new(replies.into_iter().map(Ok).collect()),
            seen: Mutex::default(),
        }
    }

    /// A reply made of text only.
    #[must_use]
    pub fn say(text: &str) -> Reply {
        Reply {
            content: text.to_string(),
            tool_calls: vec![],
        }
    }

    /// A reply that calls one tool.
    #[must_use]
    pub fn call(name: &str, arguments: Value) -> Reply {
        Reply {
            content: String::new(),
            tool_calls: vec![ToolCall {
                id: format!("call_{name}"),
                name: name.to_string(),
                arguments,
            }],
        }
    }

    /// What the model was sent, call after call.
    ///
    /// # Panics
    /// If another thread panicked while holding the record.
    #[must_use]
    pub fn seen(&self) -> Vec<Seen> {
        self.seen.lock().unwrap().clone()
    }
}

#[async_trait]
impl LlmBackend for Scripted {
    async fn chat(&self, messages: &[Message], tools: &[ToolDef]) -> Result<Reply, LlmError> {
        self.seen.lock().unwrap().push(Seen {
            messages: messages.to_vec(),
            tools: tools.to_vec(),
        });
        match self.replies.lock().unwrap().pop_front() {
            Some(Ok(reply)) => Ok(reply),
            Some(Err(e)) => Err(LlmError::Unreachable(e)),
            None => Err(LlmError::Invalid("no prepared answer left".into())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_reply_with_a_tool_call_is_read() {
        let body = json!({"choices": [{"message": {"content": null, "tool_calls": [
            {"id": "c1", "type": "function", "function": {"name": "devices_list", "arguments": "{\"silent_for_hours\": 24}"}}]}}]});
        let reply = parse_reply(&body).unwrap();
        assert_eq!(reply.content, "");
        assert_eq!(reply.tool_calls[0].name, "devices_list");
        assert_eq!(
            reply.tool_calls[0].arguments,
            json!({"silent_for_hours": 24})
        );
    }

    #[test]
    fn invalid_arguments_are_kept_as_text_and_empty_ones_are_an_object() {
        let body = json!({"choices": [{"message": {"tool_calls": [
            {"function": {"name": "a", "arguments": "{pas du json"}},
            {"function": {"name": "b", "arguments": ""}}]}}]});
        let reply = parse_reply(&body).unwrap();
        assert_eq!(reply.tool_calls[0].arguments, json!("{pas du json"));
        assert_eq!(reply.tool_calls[1].arguments, json!({}));
        assert_eq!(reply.tool_calls[1].id, "call_1");
        assert!(parse_reply(&json!({"choices": []})).is_err());
    }

    #[test]
    fn the_request_carries_the_tools_and_asks_for_no_thinking() {
        let llm = OpenAiCompat::new("http://x:8080/", "qwen", Duration::from_secs(5));
        let tools = [ToolDef {
            name: "system_status".into(),
            description: "d".into(),
            parameters: json!({"type": "object"}),
        }];
        let body = llm.body(
            &[Message::System("s".into()), Message::User("u".into())],
            &tools,
        );
        assert_eq!(body["tools"][0]["function"]["name"], "system_status");
        assert_eq!(body["chat_template_kwargs"]["enable_thinking"], false);
        assert_eq!(body["temperature"], 0);
        assert_eq!(llm.url, "http://x:8080/v1/chat/completions");
        assert!(llm.body(&[], &[]).get("tools").is_none());
    }
}
