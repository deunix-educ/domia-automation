// SPDX-License-Identifier: AGPL-3.0-or-later
//! Repository of locations (`location`).

use dominet_domain::RepoError;
use dominet_domain::location::{Location, LocationDraft, LocationId};
use dominet_domain::repo::{LocationRepo, RepoResult};

use crate::pg::repo_err;

struct LocationRow {
    id: i64,
    name: String,
    description: String,
    active: bool,
}

impl From<LocationRow> for Location {
    fn from(r: LocationRow) -> Self {
        Self {
            id: LocationId(r.id),
            name: r.name,
            description: r.description,
            active: r.active,
        }
    }
}

/// Acces PostgreSQL aux locations (implemente `LocationRepo`).
#[derive(Debug, Clone)]
pub struct PgLocations {
    pool: sqlx::PgPool,
}

impl PgLocations {
    pub(crate) fn new(pool: sqlx::PgPool) -> Self {
        Self { pool }
    }

    fn pool(&self) -> &sqlx::PgPool {
        &self.pool
    }
}

impl LocationRepo for PgLocations {
    async fn list(&self) -> RepoResult<Vec<Location>> {
        let rows = sqlx::query_as!(
            LocationRow,
            "SELECT id, name, description, active FROM location ORDER BY name, id"
        )
        .fetch_all(self.pool())
        .await
        .map_err(repo_err)?;
        Ok(rows.into_iter().map(Location::from).collect())
    }

    async fn get(&self, id: LocationId) -> RepoResult<Option<Location>> {
        let row = sqlx::query_as!(
            LocationRow,
            "SELECT id, name, description, active FROM location WHERE id = $1",
            id.0
        )
        .fetch_optional(self.pool())
        .await
        .map_err(repo_err)?;
        Ok(row.map(Location::from))
    }

    async fn create(&self, d: &LocationDraft) -> RepoResult<Location> {
        let row = sqlx::query_as!(
            LocationRow,
            "INSERT INTO location (name, description, active) VALUES ($1, $2, $3)
             RETURNING id, name, description, active",
            d.name,
            d.description,
            d.active
        )
        .fetch_one(self.pool())
        .await
        .map_err(repo_err)?;
        Ok(row.into())
    }

    async fn update(&self, id: LocationId, d: &LocationDraft) -> RepoResult<Location> {
        let row = sqlx::query_as!(
            LocationRow,
            "UPDATE location SET name = $2, description = $3, active = $4 WHERE id = $1
             RETURNING id, name, description, active",
            id.0,
            d.name,
            d.description,
            d.active
        )
        .fetch_optional(self.pool())
        .await
        .map_err(repo_err)?;
        row.map(Location::from).ok_or(RepoError::NotFound)
    }

    async fn delete(&self, id: LocationId) -> RepoResult<()> {
        let done = sqlx::query!("DELETE FROM location WHERE id = $1", id.0)
            .execute(self.pool())
            .await
            .map_err(repo_err)?;
        if done.rows_affected() == 0 {
            Err(RepoError::NotFound)
        } else {
            Ok(())
        }
    }
}
