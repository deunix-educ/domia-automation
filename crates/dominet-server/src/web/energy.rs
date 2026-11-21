// SPDX-License-Identifier: AGPL-3.0-or-later
//! `GET /api/energy/?days=N` : bilans d'energie journaliers (compte connecte), au format de l'API de l'implementation de reference
//! (`{"data": [{"date", "linky_hp_wh", "linky_hc_wh", "linky_total_wh", "solar_wh", "pac_wh", "net_wh"}]}`).
//!
//! Difference voulue : ce sont les `N` **derniers** jours (du plus ancien au plus recent) ; reference renvoyait les `N` premiers
//! enregistres (`order_by("date")[:N]`), si bien que la page se figeait au bout de trente jours.

use axum::extract::{Query, State};
use axum::routing::get;
use axum::{Json, Router};
use dominet_domain::energy::EnergyDay;
use dominet_domain::repo::EnergyRepo;
use serde::{Deserialize, Serialize};

use crate::AppState;
use crate::auth::extract::CurrentUser;
use crate::error::AppResult;

/// Plus grand nombre de jours renvoye.
pub const MAX_DAYS: u32 = 365;
/// Nombre de jours par defaut.
pub const DEFAULT_DAYS: u32 = 30;

pub fn routes() -> Router<AppState> {
    Router::new().route("/energy/", get(energy))
}

#[derive(Debug, Deserialize)]
struct Params {
    days: Option<String>,
}

/// Un jour, tel que reference le renvoyait.
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

/// Nombre de jours demande : 30 si absent ou illisible, au plus 365.
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
