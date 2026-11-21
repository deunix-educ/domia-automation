// SPDX-License-Identifier: AGPL-3.0-or-later
//! The prompts (R11 M6) and the portable output schemas, through a real MCP client.

#[macro_use]
mod common;

use common::{enabled, make_token, serve};
use rmcp::model::GetPromptRequestParams;
use serde_json::json;
use sqlx::PgPool;

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn prompts_need_the_read_scope_and_their_text_names_real_tools(pool: PgPool) {
    let st = enabled(pool);
    let reader = make_token(&st, "lecteur", &["lecture"]).await;
    let writer = make_token(&st, "configurateur", &["configuration"]).await;
    let addr = serve(&st).await;

    let client = connect!(addr, reader);
    let prompts = client.list_all_prompts().await.unwrap();
    let names: Vec<_> = prompts.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(
        names,
        ["diagnostic", "nouvelle_automation", "bilan_surveillance"]
    );

    // Every tool a prompt tells the assistant to call exists.
    let declared: Vec<&str> = dominet_mcp::SCOPES.iter().map(|(n, _)| *n).collect();
    for name in names {
        let args = json!({"need": "fermer les volets au coucher du soleil"});
        let result = client
            .get_prompt(
                GetPromptRequestParams::new(name).with_arguments(args.as_object().unwrap().clone()),
            )
            .await
            .unwrap();
        let text = format!("{:?}", result.messages);
        let mut found = 0;
        for part in text.split('`').skip(1).step_by(2) {
            let tool = part.split_whitespace().next().unwrap_or_default();
            if tool.contains('_') && !tool.contains('=') && !tool.contains('.') {
                assert!(declared.contains(&tool), "{name}: {tool}");
                found += 1;
            }
        }
        assert!(found >= 3, "{name}: {text}");
    }
    // A required argument is required; an unknown prompt is refused.
    assert!(
        client
            .get_prompt(GetPromptRequestParams::new("nouvelle_automation"))
            .await
            .is_err()
    );
    assert!(
        client
            .get_prompt(GetPromptRequestParams::new("inconnu"))
            .await
            .is_err()
    );
    client.cancel().await.unwrap();

    // Without `lecture` there is nothing to list and nothing to read.
    let client = connect!(addr, writer);
    assert!(client.list_all_prompts().await.unwrap().is_empty());
    assert!(
        client
            .get_prompt(GetPromptRequestParams::new("diagnostic"))
            .await
            .is_err()
    );
    client.cancel().await.unwrap();
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn no_output_schema_carries_a_format_clients_do_not_know(pool: PgPool) {
    let st = enabled(pool);
    let token = make_token(
        &st,
        "tout",
        &["lecture", "pilotage", "configuration", "administration"],
    )
    .await;
    let addr = serve(&st).await;
    let client = connect!(addr, token);
    let tools = client.list_all_tools().await.unwrap();
    assert!(tools.len() > 30);
    for t in tools {
        let schema = serde_json::to_string(&t.output_schema).unwrap();
        assert!(!schema.contains("uint"), "{}: {schema}", t.name);
    }
    client.cancel().await.unwrap();
}
