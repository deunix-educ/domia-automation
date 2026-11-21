// SPDX-License-Identifier: AGPL-3.0-or-later
//! Dumps the real tool catalogue and prompts as JSON for the local assistant bench (R12 A0).
//! Ignored by default: `DUMP_TOOLS_DIR=/some/dir cargo test -p dominet-mcp --test dump -- --ignored`.

#[macro_use]
mod common;

use common::{enabled, make_token, serve};
use sqlx::PgPool;

#[ignore = "writes files; run by hand"]
#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn dump_the_catalogue(pool: PgPool) {
    let dir = std::env::var("DUMP_TOOLS_DIR").expect("DUMP_TOOLS_DIR");
    let st = enabled(pool);
    let token = make_token(
        &st,
        "dump",
        &["lecture", "pilotage", "configuration", "administration"],
    )
    .await;
    let addr = serve(&st).await;
    let client = connect!(addr, token);
    let tools = client.list_all_tools().await.unwrap();
    let scopes: std::collections::BTreeMap<_, _> = dominet_mcp::SCOPES
        .iter()
        .map(|(n, s)| ((*n).to_string(), format!("{s:?}")))
        .collect();
    let out = serde_json::json!({ "tools": tools, "scopes": scopes });
    std::fs::write(
        format!("{dir}/tools.json"),
        serde_json::to_string_pretty(&out).unwrap(),
    )
    .unwrap();
    client.cancel().await.unwrap();
}
