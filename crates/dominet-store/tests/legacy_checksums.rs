// SPDX-License-Identifier: AGPL-3.0-or-later
//! A database that applied migrations 0006 and 0009 before their comments were reworded still migrates; any other checksum change
//! stays refused.

use sqlx::PgPool;

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write;
    bytes.iter().fold(String::new(), |mut out, b| {
        let _ = write!(out, "{b:02x}");
        out
    })
}

async fn stored(pool: &PgPool, version: i64) -> String {
    let bytes: Vec<u8> =
        sqlx::query_scalar("SELECT checksum FROM _sqlx_migrations WHERE version = $1")
            .bind(version)
            .fetch_one(pool)
            .await
            .unwrap();
    hex(&bytes)
}

async fn set_stored(pool: &PgPool, version: i64, checksum_hex: &str) {
    sqlx::query("UPDATE _sqlx_migrations SET checksum = decode($1, 'hex') WHERE version = $2")
        .bind(checksum_hex)
        .bind(version)
        .execute(pool)
        .await
        .unwrap();
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn a_database_migrated_before_the_rewording_still_migrates(pool: PgPool) {
    let current_6 = stored(&pool, 6).await;
    let current_9 = stored(&pool, 9).await;
    // The checksums recorded by the first deployments (text before the rewording).
    let old_6 = "ed7ba4ac644713f1053d0463df208e2beaf126be22fa326b70b47c34e10ec03b88243522b00bf45adbbc9de78c965293";
    let old_9 = "269fd46792031a8830596a0a55ef19d125456880afe80bbb222a83210ef5129c26300b1aa52b7ee84c13d3af7d030487";
    assert_ne!(
        current_6, old_6,
        "the migration text changed, so must its checksum"
    );
    set_stored(&pool, 6, old_6).await;
    set_stored(&pool, 9, old_9).await;

    dominet_store::migrate(&pool)
        .await
        .expect("legacy checksums are accepted");
    assert_eq!(stored(&pool, 6).await, current_6);
    assert_eq!(stored(&pool, 9).await, current_9);
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn any_other_checksum_change_is_still_refused(pool: PgPool) {
    let unknown = "00".repeat(48);
    set_stored(&pool, 6, &unknown).await;
    assert!(dominet_store::migrate(&pool).await.is_err());
    assert_eq!(
        stored(&pool, 6).await,
        unknown,
        "an unknown checksum is not rewritten"
    );
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn migrating_twice_changes_nothing(pool: PgPool) {
    let before = stored(&pool, 6).await;
    dominet_store::migrate(&pool).await.unwrap();
    dominet_store::migrate(&pool).await.unwrap();
    assert_eq!(stored(&pool, 6).await, before);
}
