// SPDX-License-Identifier: AGPL-3.0-or-later
//! Depots des scenes, des automations et de leur journal d'execution (R4, ADR 0031).

use std::collections::HashMap;

use chrono::{DateTime, Utc};
use dominet_domain::RepoError;
use dominet_domain::automation::{
    Automation, AutomationDraft, AutomationId, AutomationRun, RunStatus,
};
use dominet_domain::device::DeviceId;
use dominet_domain::location::LocationId;
use dominet_domain::repo::{AutomationRepo, AutomationRunRepo, RepoResult, SceneRepo};
use dominet_domain::scene::{Scene, SceneAction, SceneActionId, SceneDraft, SceneId};
use serde_json::Value;

use crate::pg::{corrupt, repo_err};

// ------------------------------------------------------------------ scenes

/// Acces PostgreSQL aux scenes.
#[derive(Debug, Clone)]
pub struct PgScenes {
    pool: sqlx::PgPool,
}

impl PgScenes {
    pub(crate) fn new(pool: sqlx::PgPool) -> Self {
        Self { pool }
    }

    /// Les actions des scenes demandees, par scene.
    async fn actions_of(&self, ids: &[i64]) -> RepoResult<HashMap<i64, Vec<SceneAction>>> {
        let rows = sqlx::query!(
            "SELECT id, scene_id, device_id, payload, position FROM scene_action WHERE scene_id = ANY($1) ORDER BY scene_id, position, id",
            ids
        )
        .fetch_all(&self.pool)
        .await
        .map_err(repo_err)?;
        let mut out: HashMap<i64, Vec<SceneAction>> = HashMap::new();
        for r in rows {
            out.entry(r.scene_id).or_default().push(SceneAction {
                id: SceneActionId(r.id),
                device: DeviceId(r.device_id),
                payload: r.payload,
                position: r.position,
            });
        }
        Ok(out)
    }

    async fn load(&self, id: i64) -> RepoResult<Option<Scene>> {
        let Some(r) = sqlx::query!(
            "SELECT id, name, icon, description, location_id, active FROM scene WHERE id = $1",
            id
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(repo_err)?
        else {
            return Ok(None);
        };
        let mut actions = self.actions_of(&[r.id]).await?;
        Ok(Some(Scene {
            id: SceneId(r.id),
            name: r.name,
            icon: r.icon,
            description: r.description,
            location: r.location_id.map(LocationId),
            active: r.active,
            actions: actions.remove(&r.id).unwrap_or_default(),
        }))
    }
}

async fn insert_actions(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    scene: i64,
    d: &SceneDraft,
) -> RepoResult<()> {
    for (position, a) in d.actions.iter().enumerate() {
        sqlx::query!(
            "INSERT INTO scene_action (scene_id, device_id, payload, position) VALUES ($1, $2, $3, $4)",
            scene,
            a.device.0,
            a.payload,
            i16::try_from(position).map_err(|_| corrupt("too many actions"))?
        )
        .execute(&mut **tx)
        .await
        .map_err(repo_err)?;
    }
    Ok(())
}

impl SceneRepo for PgScenes {
    async fn list(&self) -> RepoResult<Vec<Scene>> {
        let rows = sqlx::query!(
            "SELECT id, name, icon, description, location_id, active FROM scene ORDER BY name, id"
        )
        .fetch_all(&self.pool)
        .await
        .map_err(repo_err)?;
        let ids: Vec<i64> = rows.iter().map(|r| r.id).collect();
        let mut actions = self.actions_of(&ids).await?;
        Ok(rows
            .into_iter()
            .map(|r| Scene {
                id: SceneId(r.id),
                name: r.name,
                icon: r.icon,
                description: r.description,
                location: r.location_id.map(LocationId),
                active: r.active,
                actions: actions.remove(&r.id).unwrap_or_default(),
            })
            .collect())
    }

    async fn get(&self, id: SceneId) -> RepoResult<Option<Scene>> {
        self.load(id.0).await
    }

    async fn find_by_name(&self, name: &str) -> RepoResult<Option<Scene>> {
        let id = sqlx::query_scalar!("SELECT id FROM scene WHERE name = $1", name)
            .fetch_optional(&self.pool)
            .await
            .map_err(repo_err)?;
        match id {
            Some(id) => self.load(id).await,
            None => Ok(None),
        }
    }

    async fn create(&self, d: &SceneDraft) -> RepoResult<Scene> {
        let mut tx = self.pool.begin().await.map_err(repo_err)?;
        let id = sqlx::query_scalar!(
            "INSERT INTO scene (name, icon, description, location_id, active) VALUES ($1, $2, $3, $4, $5) RETURNING id",
            d.name,
            d.icon,
            d.description,
            d.location.map(|l| l.0),
            d.active
        )
        .fetch_one(&mut *tx)
        .await
        .map_err(repo_err)?;
        insert_actions(&mut tx, id, d).await?;
        tx.commit().await.map_err(repo_err)?;
        self.load(id).await?.ok_or(RepoError::NotFound)
    }

    async fn update(&self, id: SceneId, d: &SceneDraft) -> RepoResult<Scene> {
        let mut tx = self.pool.begin().await.map_err(repo_err)?;
        let updated = sqlx::query!(
            "UPDATE scene SET name = $2, icon = $3, description = $4, location_id = $5, active = $6 WHERE id = $1",
            id.0,
            d.name,
            d.icon,
            d.description,
            d.location.map(|l| l.0),
            d.active
        )
        .execute(&mut *tx)
        .await
        .map_err(repo_err)?;
        if updated.rows_affected() == 0 {
            return Err(RepoError::NotFound);
        }
        sqlx::query!("DELETE FROM scene_action WHERE scene_id = $1", id.0)
            .execute(&mut *tx)
            .await
            .map_err(repo_err)?;
        insert_actions(&mut tx, id.0, d).await?;
        tx.commit().await.map_err(repo_err)?;
        self.load(id.0).await?.ok_or(RepoError::NotFound)
    }

    async fn delete(&self, id: SceneId) -> RepoResult<()> {
        let done = sqlx::query!("DELETE FROM scene WHERE id = $1", id.0)
            .execute(&self.pool)
            .await
            .map_err(repo_err)?;
        if done.rows_affected() == 0 {
            Err(RepoError::NotFound)
        } else {
            Ok(())
        }
    }
}

// ------------------------------------------------------------------ automations

struct AutomationRow {
    id: i64,
    name: String,
    enabled: bool,
    description: String,
    trigger_type: String,
    trigger_config: Value,
    condition: Value,
    action_type: String,
    action_config: Value,
    script: String,
    last_triggered: Option<DateTime<Utc>>,
    trigger_count: i32,
}

impl AutomationRow {
    fn automation(self) -> RepoResult<Automation> {
        let draft = AutomationDraft::from_parts(
            &self.name,
            self.enabled,
            &self.description,
            &self.trigger_type,
            &self.trigger_config,
            &self.condition,
            &self.action_type,
            &self.action_config,
            &self.script,
        )
        .map_err(|p| corrupt(format!("invalid automation {}: {p:?}", self.id)))?;
        Ok(Automation {
            id: AutomationId(self.id),
            draft,
            last_triggered: self.last_triggered,
            trigger_count: self.trigger_count,
        })
    }
}

/// Acces PostgreSQL aux automations.
#[derive(Debug, Clone)]
pub struct PgAutomations {
    pool: sqlx::PgPool,
}

impl PgAutomations {
    pub(crate) fn new(pool: sqlx::PgPool) -> Self {
        Self { pool }
    }
}

impl AutomationRepo for PgAutomations {
    async fn list(&self) -> RepoResult<Vec<Automation>> {
        let rows = sqlx::query_as!(
            AutomationRow,
            "SELECT id, name, enabled, description, trigger_type, trigger_config, condition, action_type, action_config, script,
                    last_triggered, trigger_count FROM automation ORDER BY name, id"
        )
        .fetch_all(&self.pool)
        .await
        .map_err(repo_err)?;
        rows.into_iter().map(AutomationRow::automation).collect()
    }

    async fn list_enabled(&self) -> RepoResult<Vec<Automation>> {
        let rows = sqlx::query_as!(
            AutomationRow,
            "SELECT id, name, enabled, description, trigger_type, trigger_config, condition, action_type, action_config, script,
                    last_triggered, trigger_count FROM automation WHERE enabled ORDER BY name, id"
        )
        .fetch_all(&self.pool)
        .await
        .map_err(repo_err)?;
        rows.into_iter().map(AutomationRow::automation).collect()
    }

    async fn get(&self, id: AutomationId) -> RepoResult<Option<Automation>> {
        sqlx::query_as!(
            AutomationRow,
            "SELECT id, name, enabled, description, trigger_type, trigger_config, condition, action_type, action_config, script,
                    last_triggered, trigger_count FROM automation WHERE id = $1",
            id.0
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(repo_err)?
        .map(AutomationRow::automation)
        .transpose()
    }

    async fn create(&self, d: &AutomationDraft) -> RepoResult<Automation> {
        let p = d.parts();
        let row = sqlx::query_as!(
            AutomationRow,
            "INSERT INTO automation (name, enabled, description, trigger_type, trigger_config, condition, action_type, action_config, script)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
             RETURNING id, name, enabled, description, trigger_type, trigger_config, condition, action_type, action_config, script,
                       last_triggered, trigger_count",
            d.name,
            d.enabled,
            d.description,
            p.trigger_type,
            p.trigger_config,
            p.condition,
            p.action_type,
            p.action_config,
            d.script
        )
        .fetch_one(&self.pool)
        .await
        .map_err(repo_err)?;
        row.automation()
    }

    async fn update(&self, id: AutomationId, d: &AutomationDraft) -> RepoResult<Automation> {
        let p = d.parts();
        let row = sqlx::query_as!(
            AutomationRow,
            "UPDATE automation SET name = $2, enabled = $3, description = $4, trigger_type = $5, trigger_config = $6, condition = $7,
                    action_type = $8, action_config = $9, script = $10
             WHERE id = $1
             RETURNING id, name, enabled, description, trigger_type, trigger_config, condition, action_type, action_config, script,
                       last_triggered, trigger_count",
            id.0,
            d.name,
            d.enabled,
            d.description,
            p.trigger_type,
            p.trigger_config,
            p.condition,
            p.action_type,
            p.action_config,
            d.script
        )
        .fetch_one(&self.pool)
        .await
        .map_err(repo_err)?;
        row.automation()
    }

    async fn delete(&self, id: AutomationId) -> RepoResult<()> {
        let done = sqlx::query!("DELETE FROM automation WHERE id = $1", id.0)
            .execute(&self.pool)
            .await
            .map_err(repo_err)?;
        if done.rows_affected() == 0 {
            Err(RepoError::NotFound)
        } else {
            Ok(())
        }
    }

    async fn record_trigger(&self, id: AutomationId, at: DateTime<Utc>) -> RepoResult<()> {
        sqlx::query!(
            "UPDATE automation SET trigger_count = trigger_count + 1, last_triggered = $2 WHERE id = $1",
            id.0,
            at
        )
        .execute(&self.pool)
        .await
        .map_err(repo_err)?;
        Ok(())
    }
}

// ------------------------------------------------------------------ journal d'execution

/// Acces PostgreSQL au journal d'execution.
#[derive(Debug, Clone)]
pub struct PgAutomationRuns {
    pool: sqlx::PgPool,
}

impl PgAutomationRuns {
    pub(crate) fn new(pool: sqlx::PgPool) -> Self {
        Self { pool }
    }
}

impl AutomationRunRepo for PgAutomationRuns {
    async fn record(&self, run: &AutomationRun) -> RepoResult<()> {
        sqlx::query!(
            "INSERT INTO automation_run (automation_id, at, trigger, status, duration_ms, message) VALUES ($1, $2, $3, $4, $5, $6)",
            run.automation.0,
            run.at,
            run.trigger,
            run.status.as_str(),
            run.duration_ms,
            run.message.chars().take(1000).collect::<String>()
        )
        .execute(&self.pool)
        .await
        .map_err(repo_err)?;
        Ok(())
    }

    async fn recent(&self, automation: AutomationId, limit: i64) -> RepoResult<Vec<AutomationRun>> {
        let rows = sqlx::query!(
            "SELECT automation_id, at, trigger, status, duration_ms, message FROM automation_run
             WHERE automation_id = $1 ORDER BY at DESC, id DESC LIMIT $2",
            automation.0,
            limit
        )
        .fetch_all(&self.pool)
        .await
        .map_err(repo_err)?;
        rows.into_iter()
            .map(|r| {
                run_from(
                    r.automation_id,
                    r.at,
                    r.trigger,
                    &r.status,
                    r.duration_ms,
                    r.message,
                )
            })
            .collect()
    }

    async fn recent_all(&self, limit: i64) -> RepoResult<Vec<AutomationRun>> {
        let rows = sqlx::query!(
            "SELECT automation_id, at, trigger, status, duration_ms, message FROM automation_run
             ORDER BY at DESC, id DESC LIMIT $1",
            limit
        )
        .fetch_all(&self.pool)
        .await
        .map_err(repo_err)?;
        rows.into_iter()
            .map(|r| {
                run_from(
                    r.automation_id,
                    r.at,
                    r.trigger,
                    &r.status,
                    r.duration_ms,
                    r.message,
                )
            })
            .collect()
    }

    async fn purge_before(&self, before: DateTime<Utc>) -> RepoResult<u64> {
        let done = sqlx::query!("DELETE FROM automation_run WHERE at < $1", before)
            .execute(&self.pool)
            .await
            .map_err(repo_err)?;
        Ok(done.rows_affected())
    }
}

fn run_from(
    automation: i64,
    at: DateTime<Utc>,
    trigger: String,
    status: &str,
    duration_ms: i32,
    message: String,
) -> RepoResult<AutomationRun> {
    Ok(AutomationRun {
        automation: AutomationId(automation),
        at,
        trigger,
        status: RunStatus::parse(status).ok_or_else(|| corrupt("execution status"))?,
        duration_ms,
        message,
    })
}
