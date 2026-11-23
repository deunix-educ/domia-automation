// SPDX-License-Identifier: AGPL-3.0-or-later
//! `GET /api/energy/?days=N`: daily energy summaries (logged-in account), in the reference implementation's API format
//! (`{"data": [{"date", "linky_hp_wh", "linky_hc_wh", "linky_total_wh", "solar_wh", "pac_wh", "net_wh"}]}`).
//!
//! Intended difference: these are the **last** `N` days (from oldest to most recent); the reference returned the first `N`
//! recorded (`order_by("date")[:N]`), so the page froze after thirty days.

use axum::extract::{Query, State};
use axum::routing::get;
use axum::{Json, Router};
use dominet_domain::energy::EnergyDay;
use dominet_domain::repo::EnergyRepo;
use serde::{Deserialize, Serialize};

use crate::AppState;
use crate::auth::extract::CurrentUser;
use crate::error::AppResult;

/// Largest number of days returned.
pub const MAX_DAYS: u32 = 365;
/// Default number of days.
pub const DEFAULT_DAYS: u32 = 30;

pub fn routes() -> Router<AppState> {
    Router::new().route("/energy/", get(energy))
}

#[derive(Debug, Deserialize)]
struct Params {
    days: Option<String>,
}

/// A day, as the reference returned it.
#[derive(Debug, Serialize)]
pub struct EnergyRow {
    pub date: String,
    pub linky_hp_wh: i32,
    pub linky_hc_wh: i32,
    pub linky_total_wh: i32,
    pub solar_wh: i32,
    pub pac_wh: i32,
    pub net_wh: i32,
}

impl From<&EnergyDay> for EnergyRow {
    fn from(d: &EnergyDay) -> Self {
        Self {
            date: d.day.to_string(),
            linky_hp_wh: d.linky_hp_wh,
            linky_hc_wh: d.linky_hc_wh,
            linky_total_wh: d.linky_total_wh(),
            solar_wh: d.solar_wh,
            pac_wh: d.pac_wh,
            net_wh: d.net_wh(),
        }
    }
}

#[derive(Debug, Serialize)]
pub struct EnergyBody {
    pub data: Vec<EnergyRow>,
}

/// Number of days requested: 30 if absent or unreadable, 365 at most.
#[must_use]
pub fn days_from(text: Option<&str>) -> u32 {
    text.and_then(|t| t.trim().parse::<i64>().ok())
        .map_or(DEFAULT_DAYS, |n| {
            u32::try_from(n.clamp(0, i64::from(MAX_DAYS))).unwrap_or(0)
        })
}

async fn energy(
    State(state): State<AppState>,
    CurrentUser(_): CurrentUser,
    Query(params): Query<Params>,
) -> AppResult<Json<EnergyBody>> {
    let days = state
        .store
        .energy()
        .recent(days_from(params.days.as_deref()))
        .await?;
    Ok(Json(EnergyBody {
        data: days.iter().map(EnergyRow::from).collect(),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_number_of_days_defaults_to_thirty_and_is_capped_at_a_year() {
        assert_eq!(days_from(None), 30);
        assert_eq!(days_from(Some("abc")), 30);
        assert_eq!(days_from(Some("7")), 7);
        assert_eq!(days_from(Some(" 14 ")), 14);
        assert_eq!(days_from(Some("9999")), 365);
        assert_eq!(days_from(Some("0")), 0);
        assert_eq!(days_from(Some("-5")), 0);
    }
}
