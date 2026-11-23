// SPDX-License-Identifier: AGPL-3.0-or-later
//! Repositories of dashboards and their widgets (`dashboard`, `widget`).

use dominet_domain::RepoError;
use dominet_domain::dashboard::{
    Dashboard, DashboardDraft, DashboardId, GridRect, Presentation, Widget, WidgetDraft, WidgetId,
    WidgetType,
};
use dominet_domain::repo::{DashboardRepo, RepoResult, WidgetRepo};
use dominet_domain::user::UserId;
use uuid::Uuid;

use crate::pg::{corrupt, repo_err};

struct DashboardRow {
    id: i64,
    name: String,
    owner_id: i64,
    is_default: bool,
}

impl From<DashboardRow> for Dashboard {
    fn from(r: DashboardRow) -> Self {
        Self {
            id: DashboardId(r.id),
            name: r.name,
            owner: UserId(r.owner_id),
            is_default: r.is_default,
        }
    }
}

/// PostgreSQL access to dashboards (implements `DashboardRepo`).
#[derive(Debug, Clone)]
pub struct PgDashboards {
    pool: sqlx::PgPool,
}

impl PgDashboards {
    pub(crate) fn new(pool: sqlx::PgPool) -> Self {
        Self { pool }
    }
}

/// Locks the owner's row (serialises the default-dashboard changes of one account);
/// `NotFound` if it does not exist.
async fn lock_owner(tx: &mut sqlx::Transaction<'_, sqlx::Postgres>, owner: i64) -> RepoResult<()> {
    sqlx::query_scalar!("SELECT id FROM users WHERE id = $1 FOR UPDATE", owner)
        .fetch_optional(&mut **tx)
        .await
        .map_err(repo_err)?
        .map(|_| ())
        .ok_or(RepoError::NotFound)
}

impl DashboardRepo for PgDashboards {
    async fn list_for_owner(&self, owner: UserId) -> RepoResult<Vec<Dashboard>> {
        let rows = sqlx::query_as!(
            DashboardRow,
            "SELECT id, name, owner_id, is_default FROM dashboard WHERE owner_id = $1 ORDER BY name, id",
            owner.0
        )
        .fetch_all(&self.pool)
        .await
        .map_err(repo_err)?;
        Ok(rows.into_iter().map(Dashboard::from).collect())
    }

    async fn list_all(&self) -> RepoResult<Vec<Dashboard>> {
        let rows = sqlx::query_as!(
            DashboardRow,
            "SELECT id, name, owner_id, is_default FROM dashboard ORDER BY owner_id, name, id"
        )
        .fetch_all(&self.pool)
        .await
        .map_err(repo_err)?;
        Ok(rows.into_iter().map(Dashboard::from).collect())
    }

    async fn get(&self, id: DashboardId) -> RepoResult<Option<Dashboard>> {
        let row = sqlx::query_as!(
            DashboardRow,
            "SELECT id, name, owner_id, is_default FROM dashboard WHERE id = $1",
            id.0
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(repo_err)?;
        Ok(row.map(Dashboard::from))
    }

    async fn create(&self, owner: UserId, draft: &DashboardDraft) -> RepoResult<Dashboard> {
        let mut tx = self.pool.begin().await.map_err(repo_err)?;
        lock_owner(&mut tx, owner.0).await?;
        let existing = sqlx::query_scalar!(
            "SELECT count(*) FROM dashboard WHERE owner_id = $1",
            owner.0
        )
        .fetch_one(&mut *tx)
        .await
        .map_err(repo_err)?
        .unwrap_or(0);
        // The first dashboard of an account is its default dashboard.
        let is_default = draft.is_default || existing == 0;
        if is_default {
            sqlx::query!(
                "UPDATE dashboard SET is_default = FALSE WHERE owner_id = $1 AND is_default",
                owner.0
            )
            .execute(&mut *tx)
            .await
            .map_err(repo_err)?;
        }
        let row = sqlx::query_as!(
            DashboardRow,
            "INSERT INTO dashboard (name, owner_id, is_default) VALUES ($1, $2, $3)
             RETURNING id, name, owner_id, is_default",
            draft.name.trim(),
            owner.0,
            is_default
        )
        .fetch_one(&mut *tx)
        .await
        .map_err(repo_err)?;
        tx.commit().await.map_err(repo_err)?;
        Ok(row.into())
    }

    async fn update(&self, id: DashboardId, draft: &DashboardDraft) -> RepoResult<Dashboard> {
        let mut tx = self.pool.begin().await.map_err(repo_err)?;
        // The owner of a dashboard never changes: we read it, take the lock, THEN read the dashboard's state again.
        // Reading `is_default` before the lock would give a stale value if another change has just gone through.
        let owner_id = sqlx::query_scalar!("SELECT owner_id FROM dashboard WHERE id = $1", id.0)
            .fetch_optional(&mut *tx)
            .await
            .map_err(repo_err)?
            .ok_or(RepoError::NotFound)?;
        lock_owner(&mut tx, owner_id).await?;
        let current = sqlx::query!(
            "SELECT owner_id, is_default FROM dashboard WHERE id = $1",
            id.0
        )
        .fetch_optional(&mut *tx)
        .await
        .map_err(repo_err)?
        .ok_or(RepoError::NotFound)?;
        // A default dashboard stays so: the status moves to another dashboard when that one is designated.
        let is_default = draft.is_default || current.is_default;
        if is_default && !current.is_default {
            sqlx::query!(
                "UPDATE dashboard SET is_default = FALSE WHERE owner_id = $1 AND is_default",
                current.owner_id
            )
            .execute(&mut *tx)
            .await
            .map_err(repo_err)?;
        }
        let row = sqlx::query_as!(
            DashboardRow,
            "UPDATE dashboard SET name = $2, is_default = $3 WHERE id = $1
             RETURNING id, name, owner_id, is_default",
            id.0,
            draft.name.trim(),
            is_default
        )
        .fetch_one(&mut *tx)
        .await
        .map_err(repo_err)?;
        tx.commit().await.map_err(repo_err)?;
        Ok(row.into())
    }

    async fn delete(&self, id: DashboardId) -> RepoResult<()> {
        let done = sqlx::query!("DELETE FROM dashboard WHERE id = $1", id.0)
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

struct WidgetRow {
    id: i64,
    dashboard_id: i64,
    widget_type: String,
    title: String,
    device_id: Option<i64>,
    scene_id: Option<i64>,
    camera_uuid: Option<Uuid>,
    presentation: String,
    config: serde_json::Value,
    content: String,
    grid_col: i16,
    grid_row: i16,
    width: i16,
    height: i16,
}

impl WidgetRow {
    fn into_widget(self) -> RepoResult<Widget> {
        Ok(Widget {
            id: WidgetId(self.id),
            dashboard: DashboardId(self.dashboard_id),
            widget_type: self
                .widget_type
                .parse::<WidgetType>()
                .map_err(|e| corrupt(e.0))?,
            title: self.title,
            device_id: self.device_id,
            scene_id: self.scene_id,
            camera_uuid: self.camera_uuid,
            presentation: self
                .presentation
                .parse::<Presentation>()
                .map_err(|e| corrupt(e.0))?,
            config: self.config,
            content: self.content,
            rect: GridRect {
                col: self.grid_col,
                row: self.grid_row,
                width: self.width,
                height: self.height,
            },
        })
    }
}

/// Acces PostgreSQL aux widgets (implemente `WidgetRepo`).
#[derive(Debug, Clone)]
pub struct PgWidgets {
    pool: sqlx::PgPool,
}

impl PgWidgets {
    pub(crate) fn new(pool: sqlx::PgPool) -> Self {
        Self { pool }
    }
}

/// Locks the dashboard's row until the end of the transaction: the widget writes of one dashboard go through
/// one after the other. Without it, two simultaneous inserts on the same cells block each other in the
/// exclusion-constraint check (`deadlock detected`) instead of being cleanly refused.
/// `NotFound` if the dashboard does not exist.
async fn lock_dashboard(tx: &mut sqlx::Transaction<'_, sqlx::Postgres>, id: i64) -> RepoResult<()> {
    sqlx::query_scalar!("SELECT id FROM dashboard WHERE id = $1 FOR UPDATE", id)
        .fetch_optional(&mut **tx)
        .await
        .map_err(repo_err)?
        .map(|_| ())
        .ok_or(RepoError::NotFound)
}

/// Dashboard of a widget (it never changes), or `NotFound`.
async fn dashboard_of(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    widget: i64,
) -> RepoResult<i64> {
    sqlx::query_scalar!("SELECT dashboard_id FROM widget WHERE id = $1", widget)
        .fetch_optional(&mut **tx)
        .await
        .map_err(repo_err)?
        .ok_or(RepoError::NotFound)
}

impl WidgetRepo for PgWidgets {
    async fn list(&self, dashboard: DashboardId) -> RepoResult<Vec<Widget>> {
        let rows = sqlx::query_as!(
            WidgetRow,
            "SELECT id, dashboard_id, widget_type, title, device_id, scene_id, camera_uuid, presentation, config, content,
                    grid_col, grid_row, width, height
             FROM widget WHERE dashboard_id = $1 ORDER BY grid_row, grid_col, id",
            dashboard.0
        )
        .fetch_all(&self.pool)
        .await
        .map_err(repo_err)?;
        rows.into_iter().map(WidgetRow::into_widget).collect()
    }

    async fn get(&self, id: WidgetId) -> RepoResult<Option<Widget>> {
        sqlx::query_as!(
            WidgetRow,
            "SELECT id, dashboard_id, widget_type, title, device_id, scene_id, camera_uuid, presentation, config, content,
                    grid_col, grid_row, width, height
             FROM widget WHERE id = $1",
            id.0
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(repo_err)?
        .map(WidgetRow::into_widget)
        .transpose()
    }

    async fn create(&self, dashboard: DashboardId, d: &WidgetDraft) -> RepoResult<Widget> {
        let mut tx = self.pool.begin().await.map_err(repo_err)?;
        lock_dashboard(&mut tx, dashboard.0).await?;
        let row = sqlx::query_as!(
            WidgetRow,
            "INSERT INTO widget (dashboard_id, widget_type, title, device_id, scene_id, camera_uuid, presentation, config,
                                 content, grid_col, grid_row, width, height)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13)
             RETURNING id, dashboard_id, widget_type, title, device_id, scene_id, camera_uuid, presentation, config, content,
                       grid_col, grid_row, width, height",
            dashboard.0,
            d.widget_type.as_str(),
            d.title.trim(),
            d.device_id,
            d.scene_id,
            d.camera_uuid,
            d.presentation.as_str(),
            d.config,
            d.content,
            d.rect.col,
            d.rect.row,
            d.rect.width,
            d.rect.height,
        )
        .fetch_one(&mut *tx)
        .await
        .map_err(repo_err)?;
        tx.commit().await.map_err(repo_err)?;
        row.into_widget()
    }

    async fn update(&self, id: WidgetId, d: &WidgetDraft) -> RepoResult<Widget> {
        let mut tx = self.pool.begin().await.map_err(repo_err)?;
        let dashboard = dashboard_of(&mut tx, id.0).await?;
        lock_dashboard(&mut tx, dashboard).await?;
        let row = sqlx::query_as!(
            WidgetRow,
            "UPDATE widget SET widget_type = $2, title = $3, device_id = $4, scene_id = $5, camera_uuid = $6,
                    presentation = $7, config = $8, content = $9, grid_col = $10, grid_row = $11, width = $12, height = $13
             WHERE id = $1
             RETURNING id, dashboard_id, widget_type, title, device_id, scene_id, camera_uuid, presentation, config, content,
                       grid_col, grid_row, width, height",
            id.0,
            d.widget_type.as_str(),
            d.title.trim(),
            d.device_id,
            d.scene_id,
            d.camera_uuid,
            d.presentation.as_str(),
            d.config,
            d.content,
            d.rect.col,
            d.rect.row,
            d.rect.width,
            d.rect.height,
        )
        .fetch_optional(&mut *tx)
        .await
        .map_err(repo_err)?
        .ok_or(RepoError::NotFound)?;
        tx.commit().await.map_err(repo_err)?;
        row.into_widget()
    }

    async fn move_to(&self, id: WidgetId, col: i16, row: i16) -> RepoResult<Widget> {
        let mut tx = self.pool.begin().await.map_err(repo_err)?;
        let dashboard = dashboard_of(&mut tx, id.0).await?;
        lock_dashboard(&mut tx, dashboard).await?;
        let moved = sqlx::query_as!(
            WidgetRow,
            "UPDATE widget SET grid_col = $2, grid_row = $3 WHERE id = $1
             RETURNING id, dashboard_id, widget_type, title, device_id, scene_id, camera_uuid, presentation, config, content,
                       grid_col, grid_row, width, height",
            id.0,
            col,
            row
        )
        .fetch_optional(&mut *tx)
        .await
        .map_err(repo_err)?
        .ok_or(RepoError::NotFound)?;
        tx.commit().await.map_err(repo_err)?;
        moved.into_widget()
    }

    async fn delete(&self, id: WidgetId) -> RepoResult<()> {
        let done = sqlx::query!("DELETE FROM widget WHERE id = $1", id.0)
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
