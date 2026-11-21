// SPDX-License-Identifier: AGPL-3.0-or-later
//! The agent against the real tool server (test database) and a scripted model: no GPU, no model.

use std::sync::Arc;
use std::time::Duration;

use dominet_agent::llm::Seen;
use dominet_agent::{Agent, AgentError, Bridge, Limits, LlmBackend, Message, Regime, Scripted};
use dominet_domain::device::DeviceDraft;
use dominet_domain::mcp::{Actor, AuditKind};
use dominet_domain::repo::{DeviceRepo, McpAuditRepo};
use dominet_domain::user::{Role, UserId};
use dominet_server::AppState;
use dominet_server::auth::password::Hasher;
use dominet_server::config::Config;
use serde_json::json;
use sqlx::PgPool;

fn state(pool: PgPool) -> AppState {
    let vars = [
        ("DATABASE_URL", "postgres://u:p@127.0.0.1:1/none"),
        ("COOKIE_SECURE", "false"),
    ];
    let config = Config::from_lookup(|k| {
        vars.iter()
            .find(|(key, _)| *key == k)
            .map(|(_, v)| (*v).to_string())
    })
    .unwrap();
    AppState::with_hasher(
        pool,
        Arc::new(config),
        Hasher::with_params(1024, 1, 1).unwrap(),
    )
}

fn admin() -> Actor {
    Actor::Session {
        user: UserId(1),
        username: "admin".into(),
        role: Role::Admin,
    }
}

async fn agent(
    pool: PgPool,
    regime: Regime,
    script: Scripted,
    limits: Limits,
) -> (AppState, Agent<Scripted>) {
    let st = state(pool);
    let bridge = Bridge::connect(st.clone(), admin(), regime).await.unwrap();
    (st.clone(), Agent::new(script, bridge, limits))
}

fn last_tool_message(seen: &Seen) -> String {
    seen.messages
        .iter()
        .rev()
        .find_map(|m| match m {
            Message::Tool { content, .. } => Some(content.clone()),
            _ => None,
        })
        .expect("a tool message")
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn a_read_runs_and_its_result_reaches_the_model_as_data(pool: PgPool) {
    let script = Scripted::new([
        Scripted::call("system_status", json!({})),
        Scripted::say("Tout va bien."),
    ]);
    let (_st, agent) = agent(pool, Regime::Daily, script, Limits::default()).await;

    let turn = agent.run(&[], "Comment va la maison ?").await.unwrap();
    assert_eq!(turn.reply, "Tout va bien.");
    assert!(turn.cards.is_empty());
    assert_eq!(turn.tool_calls, 1);
    // user, assistant (call), tool, assistant (answer)
    assert_eq!(turn.messages.len(), 4);

    let seen = agent.llm().seen();
    assert_eq!(seen.len(), 2);
    let data = last_tool_message(&seen[1]);
    assert!(data.starts_with("[DONNEES"), "{data}");
    assert!(data.contains("\"database\""), "{data}");
    assert!(data.ends_with("[FIN DES DONNEES]"));
    // The system prompt is always first and identical, so that the model server can cache the prefix.
    assert_eq!(seen[0].messages[0], seen[1].messages[0]);
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn an_action_becomes_a_card_and_nothing_is_done_or_written(pool: PgPool) {
    let script = Scripted::new([Scripted::call(
        "device_command",
        json!({"device": "Volet-salon", "command": {"state": "CLOSE"}, "confirm": true, "dry_run": false}),
    )]);
    let (st, agent) = agent(pool, Regime::Daily, script, Limits::default()).await;
    st.store
        .devices()
        .create(&DeviceDraft::new("Volet-salon"))
        .await
        .unwrap();

    let turn = agent.run(&[], "Ferme le volet du salon.").await.unwrap();
    assert_eq!(turn.cards.len(), 1, "{turn:?}");
    let card = &turn.cards[0];
    assert_eq!(card.tool, "device_command");
    // The model's own `confirm` and `dry_run` are dropped: they never reach the server.
    assert_eq!(
        card.arguments,
        json!({"device": "Volet-salon", "command": {"state": "CLOSE"}})
    );
    assert!(card.summary.contains("Volet-salon"), "{}", card.summary);
    // A simulation leaves no trace in the audit trail and the turn ends: the user decides.
    assert!(
        st.store
            .mcp_audit()
            .recent(Some(AuditKind::ToolCall), 10)
            .await
            .unwrap()
            .is_empty()
    );
    assert_eq!(agent.llm().seen().len(), 1);
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn each_regime_offers_its_tools_and_hides_confirm_and_dry_run(pool: PgPool) {
    let st = state(pool);
    let daily = Bridge::connect(st.clone(), admin(), Regime::Daily)
        .await
        .unwrap();
    let config = Bridge::connect(st.clone(), admin(), Regime::Config)
        .await
        .unwrap();
    let names = |b: &Bridge| {
        b.tools()
            .into_iter()
            .map(|t| t.name)
            .collect::<std::collections::BTreeSet<_>>()
    };
    let (d, c) = (names(&daily), names(&config));
    assert!(d.contains("devices_list") && d.contains("device_command"));
    assert!(!d.contains("config_upsert") && !d.contains("setting_set"));
    assert!(c.contains("config_upsert") && c.contains("script_set") && c.contains("config_schema"));
    assert!(!c.contains("device_command") && !c.contains("logs_query"));
    assert_eq!(d.len(), 29, "{d:?}");

    for tool in daily.tools().into_iter().chain(config.tools()) {
        let text = tool.parameters.to_string();
        let props = tool.parameters.get("properties");
        for hidden in ["confirm", "dry_run"] {
            assert!(
                props.is_none_or(|p| p.get(hidden).is_none()),
                "{} montre {hidden}",
                tool.name
            );
        }
        assert!(
            !text.contains("\"format\"") && !text.contains("$schema"),
            "{text}"
        );
    }
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn a_tool_outside_the_regime_is_refused_and_the_model_is_told(pool: PgPool) {
    let st = state(pool);
    let bridge = Bridge::connect(st, admin(), Regime::Daily).await.unwrap();
    let err = bridge.call("config_upsert", &json!({})).await.unwrap_err();
    assert!(err.to_string().contains("n'existe pas"), "{err}");

    // Through the agent, the refusal is data: the model may explain or try again.
    let script = Scripted::new([
        Scripted::call("setting_set", json!({"key": "X", "value": "y"})),
        Scripted::say("Je ne peux pas faire cela ici."),
    ]);
    let agent = Agent::new(script, bridge, Limits::default());
    let turn = agent.run(&[], "Change un reglage").await.unwrap();
    assert!(turn.cards.is_empty());
    assert_eq!(turn.reply, "Je ne peux pas faire cela ici.");
    let seen = agent.llm().seen();
    assert!(last_tool_message(&seen[1]).contains("n'existe pas ou n'est pas disponible"));
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn the_limits_of_a_turn_are_enforced(pool: PgPool) {
    let script = Scripted::new(
        std::iter::repeat_with(|| Scripted::call("system_status", json!({}))).take(5),
    );
    let limits = Limits {
        max_tool_calls: 3,
        ..Limits::default()
    };
    let (_st, agent) = agent(pool, Regime::Daily, script, limits).await;
    let err = agent.run(&[], "boucle").await.unwrap_err();
    assert!(matches!(err, AgentError::TooManyCalls(3)), "{err}");
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn a_model_that_does_not_answer_in_time_is_cut_off(pool: PgPool) {
    struct Slow;
    #[async_trait::async_trait]
    impl LlmBackend for Slow {
        async fn chat(
            &self,
            _: &[Message],
            _: &[dominet_agent::ToolDef],
        ) -> Result<dominet_agent::Reply, dominet_agent::LlmError> {
            tokio::time::sleep(Duration::from_secs(30)).await;
            Ok(dominet_agent::Reply::default())
        }
    }
    let st = state(pool);
    let bridge = Bridge::connect(st, admin(), Regime::Daily).await.unwrap();
    let agent = Agent::new(
        Slow,
        bridge,
        Limits {
            deadline: Duration::from_millis(200),
            ..Limits::default()
        },
    );
    assert!(matches!(
        agent.run(&[], "x").await.unwrap_err(),
        AgentError::Timeout(_)
    ));
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn a_card_changes_something_only_when_the_user_confirms_it(pool: PgPool) {
    use dominet_domain::repo::LocationRepo;

    let script = Scripted::new([Scripted::call(
        "config_upsert",
        json!({"kind": "location", "data": {"name": "Cave", "description": "sous-sol"}}),
    )]);
    let st = state(pool);
    let bridge = Bridge::connect(st.clone(), admin(), Regime::Config)
        .await
        .unwrap();
    let agent = Agent::new(script, bridge, Limits::default());

    let turn = agent.run(&[], "Cree l'emplacement Cave.").await.unwrap();
    assert_eq!(turn.cards.len(), 1, "{turn:?}");
    // The proposal wrote nothing.
    assert!(st.store.locations().list().await.unwrap().is_empty());

    // The click: a fresh link, as the interface makes it, runs the card for real.
    let bridge = Bridge::connect(st.clone(), admin(), Regime::Config)
        .await
        .unwrap();
    let answer = bridge.confirm(&turn.cards[0]).await.unwrap();
    assert!(answer.contains("Cave"), "{answer}");
    let rows = st.store.locations().list().await.unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].name, "Cave");
    // Now it is in the audit trail.
    assert_eq!(
        st.store
            .mcp_audit()
            .recent(Some(AuditKind::ToolCall), 10)
            .await
            .unwrap()
            .len(),
        1
    );
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn a_read_cannot_be_run_as_a_confirmed_card(pool: PgPool) {
    let st = state(pool);
    let bridge = Bridge::connect(st, admin(), Regime::Daily).await.unwrap();
    let card = dominet_agent::ActionCard {
        tool: "devices_list".into(),
        arguments: json!({}),
        summary: String::new(),
        before: None,
        after: None,
    };
    assert!(bridge.confirm(&card).await.is_err());
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn a_name_said_loosely_finds_the_device_and_a_wrong_one_lists_the_real_names(pool: PgPool) {
    let st = state(pool);
    for name in ["Prise-cafetiere", "Volet-salon", "Volet-chambre"] {
        st.store
            .devices()
            .create(&DeviceDraft::new(name))
            .await
            .unwrap();
    }
    let bridge = Bridge::connect(st, admin(), Regime::Daily).await.unwrap();
    // « cafetière », with its accent and without the "Prise-", is the one device that fits.
    let dominet_agent::Outcome::Card(card) = bridge
        .call(
            "device_command",
            &json!({"device": "cafetière", "command": {"state": "ON"}}),
        )
        .await
        .unwrap()
    else {
        panic!("a card was expected")
    };
    assert!(card.summary.contains("Prise-cafetiere"), "{}", card.summary);
    // « volet » fits two: nothing is guessed, the model is told which exist (the agent hands this error to the model as data).
    let text = bridge
        .call(
            "device_command",
            &json!({"device": "volet", "command": {"state": "OPEN"}}),
        )
        .await
        .unwrap_err()
        .to_string();
    assert!(
        text.contains("Volet-salon") && text.contains("Volet-chambre"),
        "{text}"
    );
    // An unknown one lists what exists.
    let text = bridge
        .call(
            "device_command",
            &json!({"device": "garage", "command": {"state": "ON"}}),
        )
        .await
        .unwrap_err()
        .to_string();
    assert!(
        text.contains("Existing names") && text.contains("Prise-cafetiere"),
        "{text}"
    );
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn a_promise_without_a_tool_call_is_reminded_once(pool: PgPool) {
    // The model says it will look, then does it after the reminder.
    let script = Scripted::new([
        Scripted::say("Je vais chercher l'etat de la maison."),
        Scripted::call("system_status", json!({})),
        Scripted::say("Tout va bien."),
    ]);
    let (_st, first) = agent(pool.clone(), Regime::Daily, script, Limits::default()).await;
    let turn = first.run(&[], "Comment va la maison ?").await.unwrap();
    assert_eq!((turn.reply.as_str(), turn.tool_calls), ("Tout va bien.", 1));
    let seen = first.llm().seen();
    assert!(
        matches!(seen[1].messages.last(), Some(Message::User(m)) if m.starts_with("Appelle maintenant")),
        "{:?}",
        seen[1].messages.last()
    );

    // A second promise is not chased: only one reminder per turn.
    let script = Scripted::new([
        Scripted::say("Je vais chercher."),
        Scripted::say("Je cherche encore."),
    ]);
    let (_st, second) = agent(pool, Regime::Daily, script, Limits::default()).await;
    let turn = second.run(&[], "Fais quelque chose.").await.unwrap();
    assert_eq!(turn.reply, "Je cherche encore.");
    assert_eq!(second.llm().seen().len(), 2);
}
