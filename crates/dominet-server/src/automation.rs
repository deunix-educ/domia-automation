// SPDX-License-Identifier: AGPL-3.0-or-later
//! Server-side effects of the automations (R4 lot 3, ADR 0032): publish on the broker, activate a scene, notify the interface.

// The port's methods are `async` for the executors that wait (scenes); this one waits for nothing.
#![allow(clippy::unused_async_trait_impl)]

use dominet_automation::engine::Executor;
use dominet_automation::script::DeviceView;
use dominet_domain::device::{DeviceId, LogEntry};
use dominet_domain::repo::{DeviceFilter, DeviceRepo, StateLogRepo};
use dominet_domain::scene::SceneId;
use dominet_proto::{NotificationLevel, SceneOutcome, Topic, WsServerMsg};
use serde_json::Value;

use crate::AppState;

/// Engine executor. `state.mqtt` must be the `mqtt` role's client (the engine runs with it).
#[derive(Debug, Clone)]
pub struct ServerExecutor {
    state: AppState,
}

impl ServerExecutor {
    #[must_use]
    pub fn new(state: AppState) -> Self {
        Self { state }
    }
}

/// Text of a scene activation's result: `Ok` if everything went out.
fn scene_result(outcome: &SceneOutcome) -> Result<String, String> {
    match outcome {
        SceneOutcome::NotFound => Err("scene not found or disabled".to_string()),
        SceneOutcome::Activated { sent, failed } if failed.is_empty() => {
            Ok(format!("scene activated: {sent} command(s) sent"))
        }
        SceneOutcome::Activated { sent, failed } => Err(format!(
            "scene activated with {} failure(s) ({sent} sent): {}",
            failed.len(),
            failed
                .iter()
                .map(|f| format!("{} {:?}", f.device, f.outcome))
                .collect::<Vec<_>>()
                .join(", ")
        )),
    }
}

impl Executor for ServerExecutor {
    async fn publish(&self, topic: &str, payload: &str) -> Result<String, String> {
        let Some(client) = &self.state.mqtt else {
            return Err("source unavailable (no link with the broker)".to_string());
        };
        client
            .publish(topic, payload, 1, false)
            .map(|()| format!("published on {topic}"))
            .map_err(|e| format!("publication refused: {e}"))
    }

    async fn scene(&self, id: SceneId) -> Result<String, String> {
        scene_result(&crate::scenes::activate_by_id(&self.state, id).await)
    }

    async fn scene_by_name(&self, name: &str) -> Result<String, String> {
        scene_result(&crate::scenes::activate_by_name(&self.state, name).await)
    }

    async fn log_value(&self, device: i64, key: &str, value: &Value) -> Result<(), String> {
        let entry = match value {
            Value::Number(n) => LogEntry {
                key: key.to_string(),
                value_num: n.as_f64(),
                value_str: String::new(),
            },
            Value::Bool(b) => LogEntry {
                key: key.to_string(),
                value_num: Some(f64::from(u8::from(*b))),
                value_str: if *b { "True" } else { "False" }.to_string(),
            },
            Value::String(s) => LogEntry {
                key: key.to_string(),
                value_num: None,
                value_str: s.chars().take(128).collect(),
            },
            other => LogEntry {
                key: key.to_string(),
                value_num: None,
                value_str: other.to_string().chars().take(128).collect(),
            },
        };
        let now = chrono::Utc::now();
        self.state
            .store
            .state_log()
            .record(DeviceId(device), now, &[entry])
            .await
            .map(|_| ())
            .map_err(|e| format!("history refused: {e}"))
    }

    async fn devices(&self) -> Result<Vec<DeviceView>, String> {
        let all = self
            .state
            .store
            .devices()
            .list(&DeviceFilter {
                active_only: true,
                ..DeviceFilter::default()
            })
            .await
            .map_err(|e| format!("unreadable devices: {e}"))?;
        let now = chrono::Utc::now();
        Ok(all.iter().map(|d| DeviceView::of(d, now)).collect())
    }

    fn notify(&self, message: &str, sms: bool) {
        // Banner for all the logged-in accounts, then e-mail to the alerts recipient and Free Mobile SMS if the automation
        // asks for it (plan R5, D7). Sending is done separately: an automation never waits for SMTP.
        tracing::info!(sms, %message, "notification d'automation");
        let (channels, text) = (self.state.channels.clone(), message.to_string());
        tokio::spawn(async move {
            channels.notify(&text, sms).await;
        });
        self.state.hub.publish(
            &Topic::Notifications,
            WsServerMsg::Notification {
                level: NotificationLevel::Info,
                text: message.chars().take(200).collect(),
            },
        );
    }
}

#[cfg(test)]
mod tests {
    use dominet_proto::{CommandOutcome, SceneFailureDto};

    use super::*;

    #[test]
    fn a_scene_is_a_success_only_when_every_command_went_out() {
        assert_eq!(
            scene_result(&SceneOutcome::Activated {
                sent: 4,
                failed: vec![]
            }),
            Ok("scene activated: 4 command(s) sent".to_string())
        );
        let partial = scene_result(&SceneOutcome::Activated {
            sent: 3,
            failed: vec![SceneFailureDto {
                device: "Volet".into(),
                outcome: CommandOutcome::NotFound,
            }],
        });
        assert!(
            partial
                .unwrap_err()
                .contains("1 failure(s) (3 sent): Volet NotFound")
        );
        assert!(scene_result(&SceneOutcome::NotFound).is_err());
    }
}
