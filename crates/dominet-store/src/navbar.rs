// SPDX-License-Identifier: AGPL-3.0-or-later
//! Repository of the navigation (`navbar_item`).

use dominet_domain::RepoError;
use dominet_domain::navbar::{NavPosition, NavbarDraft, NavbarItem, NavbarItemId};
use dominet_domain::repo::{NavbarRepo, RepoResult};

use crate::pg::{corrupt, repo_err};

struct NavbarRow {
    id: i64,
    icon_name: String,
    label: String,
    url: String,
    url_target: String,
    admin_only: bool,
    position: String,
    sort_order: i16,
}

impl NavbarRow {
    fn into_item(self) -> RepoResult<NavbarItem> {
        let position = self
            .position
            .parse::<NavPosition>()
            .map_err(|_| corrupt(format!("position \"{}\"", self.position)))?;
        Ok(NavbarItem {
            id: NavbarItemId(self.id),
            icon_name: self.icon_name,
            label: self.label,
            url: self.url,
            url_target: self.url_target,
            admin_only: self.admin_only,
            position,
            sort_order: self.sort_order,
        })
    }
}

/// Acces PostgreSQL aux navbar (implemente `NavbarRepo`).
#[derive(Debug, Clone)]
pub struct PgNavbar {
    pool: sqlx::PgPool,
}

impl PgNavbar {
    pub(crate) fn new(pool: sqlx::PgPool) -> Self {
        Self { pool }
    }

    fn pool(&self) -> &sqlx::PgPool {
        &self.pool
    }
}

impl NavbarRepo for PgNavbar {
    async fn list(&self) -> RepoResult<Vec<NavbarItem>> {
        // Top before bottom ('bottom' < 'top' alphabetically, hence the expression), then order, then label.
        sqlx::query_as!(
            NavbarRow,
            "SELECT id, icon_name, label, url, url_target, admin_only, position, sort_order
             FROM navbar_item ORDER BY (position = 'bottom'), sort_order, label"
        )
        .fetch_all(self.pool())
        .await
        .map_err(repo_err)?
        .into_iter()
        .map(NavbarRow::into_item)
        .collect()
    }

    async fn get(&self, id: NavbarItemId) -> RepoResult<Option<NavbarItem>> {
        sqlx::query_as!(
            NavbarRow,
            "SELECT id, icon_name, label, url, url_target, admin_only, position, sort_order
             FROM navbar_item WHERE id = $1",
            id.0
        )
        .fetch_optional(self.pool())
        .await
        .map_err(repo_err)?
        .map(NavbarRow::into_item)
        .transpose()
    }

    async fn create(&self, d: &NavbarDraft) -> RepoResult<NavbarItem> {
        sqlx::query_as!(
            NavbarRow,
            "INSERT INTO navbar_item (icon_name, label, url, url_target, admin_only, position, sort_order)
             VALUES ($1, $2, $3, $4, $5, $6, $7)
             RETURNING id, icon_name, label, url, url_target, admin_only, position, sort_order",
            d.icon_name,
            d.label,
            d.url,
            d.url_target,
            d.admin_only,
            d.position.as_str(),
            d.sort_order,
        )
        .fetch_one(self.pool())
        .await
        .map_err(repo_err)?
        .into_item()
    }

    async fn update(&self, id: NavbarItemId, d: &NavbarDraft) -> RepoResult<NavbarItem> {
        sqlx::query_as!(
            NavbarRow,
            "UPDATE navbar_item SET icon_name = $2, label = $3, url = $4, url_target = $5,
                 admin_only = $6, position = $7, sort_order = $8
             WHERE id = $1
             RETURNING id, icon_name, label, url, url_target, admin_only, position, sort_order",
            id.0,
            d.icon_name,
            d.label,
            d.url,
            d.url_target,
            d.admin_only,
            d.position.as_str(),
            d.sort_order,
        )
        .fetch_optional(self.pool())
        .await
        .map_err(repo_err)?
        .ok_or(RepoError::NotFound)?
        .into_item()
    }

    async fn delete(&self, id: NavbarItemId) -> RepoResult<()> {
        let done = sqlx::query!("DELETE FROM navbar_item WHERE id = $1", id.0)
            .execute(self.pool())
            .await
            .map_err(repo_err)?;
        if done.rows_affected() == 0 {
            Err(RepoError::NotFound)
        } else {
            Ok(())
        }
    }

    async fn insert_if_absent(&self, d: &NavbarDraft) -> RepoResult<bool> {
        let done = sqlx::query!(
            "INSERT INTO navbar_item (icon_name, label, url, url_target, admin_only, position, sort_order)
             SELECT $1, $2, $3, $4, $5, $6, $7
             WHERE NOT EXISTS (SELECT 1 FROM navbar_item WHERE position = $6 AND label = $2)",
            d.icon_name,
            d.label,
            d.url,
            d.url_target,
            d.admin_only,
            d.position.as_str(),
            d.sort_order,
        )
        .execute(self.pool())
        .await
        .map_err(repo_err)?;
        Ok(done.rows_affected() == 1)
    }
}
