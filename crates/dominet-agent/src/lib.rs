// SPDX-License-Identifier: AGPL-3.0-or-later
//! Local assistant of DomiNet (R12, ADR 0060): an agent loop between a self-hosted LLM and the MCP tools of the application.
//!
//! * [`llm`]: the [`LlmBackend`] trait, the `llama-server` implementation (OpenAI-compatible API) and a scripted fake for tests;
//! * [`bridge`]: an in-memory MCP client on the real tool server, acting as the signed-in account; tool schemas shown to the
//!   model, `confirm` and `dry_run` hidden; every tool of an action scope is called in `dry_run` and becomes a card;
//! * [`profile`]: the tools each regime (daily, configuration) may use;
//! * [`agent`]: the loop and its limits.
//!
//! **The model never changes anything**: a tool outside the `lecture` scope is only ever called with `dry_run = true`.

pub mod agent;
pub mod bridge;
pub mod changes;
pub mod diff;
pub mod llm;
pub mod profile;
pub mod prompts;
pub mod service;

pub use agent::{Agent, AgentError, Limits, Turn};
pub use bridge::{ActionCard, Bridge, BridgeError, Outcome};
pub use llm::{LlmBackend, LlmError, Message, OpenAiCompat, Reply, Scripted, ToolCall, ToolDef};
pub use profile::Regime;
pub use service::{AgentHub, Event, HubError, Polled, hub, test_connection};
