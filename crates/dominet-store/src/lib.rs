// SPDX-License-Identifier: AGPL-3.0-or-later
//! Storage adapters: PostgreSQL migrations and `sqlx` repositories (the ports are in `dominet-domain`).
//!
//! Queries are checked at compile time (`query!`); the offline metadata is in `.sqlx/`
//! (voir `make sqlx-prepare`).

mod alerts;
mod android;
mod attempts;
mod automation;
mod cameras;
mod dashboards;
mod devices;
mod locations;
mod mcp;
mod navbar;
mod observability;
mod pg;
pub mod reduct;
mod schedule;
mod sessions;
mod settings;
mod sources;
mod stream_access;
mod users;
pub mod video;

use sqlx::PgPool;
use sqlx::migrate::{MigrateError, Migrator};

pub use alerts::{PgAlertRules, PgAlerts, PgEnergy};
pub use android::{PgAndroidDevices, PgCalls, PgMessages, PgSms, PgSmsOutbox};
pub use attempts::PgAttempts;
pub use automation::{PgAutomationRuns, PgAutomations, PgScenes};
pub use cameras::{
    PgCameraEvents, PgCameras, PgDetectorConfig, PgPtzPresets, PgRecordings, PgSnapshots,
};
pub use dashboards::{PgDashboards, PgWidgets};
pub use devices::{PgDeviceGroups, PgDeviceImages, PgDeviceLabels, PgDevices, PgStateLog};
pub use locations::PgLocations;
pub use mcp::{PgMcpAudit, PgMcpTokens};
pub use navbar::PgNavbar;
pub use observability::{PgGrafanaRules, PgPrometheusRules};
pub use pg::PgStore;
pub use schedule::PgSchedule;
pub use sessions::{PgSessionStore, USER_ID_KEY};
pub use settings::PgSettings;
pub use sources::{PgMqttSources, PgWeather};
pub use stream_access::PgStreamAccess;
pub use users::PgUsers;

/// Migrations embedded in the binary (`migrations/` folder).
pub static MIGRATOR: Migrator = sqlx::migrate!("./migrations");

/// Applies the pending migrations; running again has no effect.
///
/// # Errors
/// Returns the `sqlx` error (connection, invalid SQL, migration modified after being applied...).
pub async fn migrate(pool: &PgPool) -> Result<(), MigrateError> {
    accept_legacy_checksums(pool).await?;
    MIGRATOR.run(pool).await
}

/// Migrations whose text was reworded (comments only) after databases had already applied them: `(version, SHA-384 of the text as it
/// was applied, in hex)`. `sqlx` refuses a migration whose checksum changed; for these exact old checksums the stored value is replaced
/// by the current one, so a database created before the rewording still migrates. Any other checksum stays an error.
const LEGACY_CHECKSUMS: [(i64, &str); 2] = [
    (
        6,
        "ed7ba4ac644713f1053d0463df208e2beaf126be22fa326b70b47c34e10ec03b88243522b00bf45adbbc9de78c965293",
    ),
    (
        9,
        "269fd46792031a8830596a0a55ef19d125456880afe80bbb222a83210ef5129c26300b1aa52b7ee84c13d3af7d030487",
    ),
];

async fn accept_legacy_checksums(pool: &PgPool) -> Result<(), MigrateError> {
    let tracked: bool = sqlx::query_scalar("SELECT to_regclass('_sqlx_migrations') IS NOT NULL")
        .fetch_one(pool)
        .await?;
    if !tracked {
        return Ok(());
    }
    for (version, old_hex) in LEGACY_CHECKSUMS {
        let Some(current) = MIGRATOR.iter().find(|m| m.version == version) else {
            continue;
        };
        sqlx::query(
            "UPDATE _sqlx_migrations SET checksum = $1 \
             WHERE version = $2 AND checksum = decode($3, 'hex')",
        )
        .bind(current.checksum.as_ref())
        .bind(version)
        .bind(old_hex)
        .execute(pool)
        .await?;
    }
    Ok(())
}
