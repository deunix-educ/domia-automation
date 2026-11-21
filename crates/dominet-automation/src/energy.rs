// SPDX-License-Identifier: AGPL-3.0-or-later
//! Bilan energetique d'une journee (R4 lot 7, ADR 0036) : `compute_daily_energy` de l'implementation de reference.
//!
//! Le jour est le **jour local** (du minuit local au minuit local suivant, 23 ou 25 h les jours de changement d'heure), les
//! compteurs du Linky (`HCHP`, `HCHC`) et de la pompe a chaleur (`energy_a` + `energy_b`) sont cumulatifs (maximum - minimum du jour),
//! la production solaire est le maximum de `qdy` (kWh remis a zero chaque jour) converti en Wh. Un appareil introuvable compte zero.

use chrono::{DateTime, Duration, NaiveDate, Utc};
use chrono_tz::Tz;
use dominet_domain::energy::{EnergyDay, cumulative_delta, solar_wh_from_qdy};
use dominet_domain::repo::{DeviceRepo, EnergyRepo, RepoResult, StateLogRepo};

use crate::next::to_utc;

/// Noms (`friendly_name`) des appareils du bilan : `ENERGY_LINKY_DEVICE`, `ENERGY_SOLAR_DEVICE`, `ENERGY_PAC_DEVICE`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnergyDevices {
    pub linky: String,
    pub solar: String,
    pub pac: String,
}

impl Default for EnergyDevices {
    /// Les noms de la production.
    fn default() -> Self {
        Self {
            linky: "consommation".into(),
            solar: "prod".into(),
            pac: "PAC-CE".into(),
        }
    }
}

/// Cles d'etat qu'il faut **suivre** (historique) : celles du bilan journalier (`HCHP`, `HCHC`, `qdy`, `energy_a`,
/// `energy_b`) et les puissances instantanees du tableau Grafana « Energie » (`PAPP`, `pac`, `power_ab`).
/// Une cle vide est ignoree.
#[must_use]
pub fn required_keys(devices: &EnergyDevices) -> [(String, [&'static str; 3]); 3] {
    [
        (devices.linky.clone(), ["HCHP", "HCHC", "PAPP"]),
        (devices.solar.clone(), ["qdy", "pac", ""]),
        (devices.pac.clone(), ["energy_a", "energy_b", "power_ab"]),
    ]
}

/// Intervalle `[debut, fin[` (UTC) du jour local `day`.
#[must_use]
pub fn day_window(tz: Tz, day: NaiveDate) -> Option<(DateTime<Utc>, DateTime<Utc>)> {
    let start = to_utc(tz, day.and_hms_opt(0, 0, 0)?)?;
    let end = to_utc(
        tz,
        day.checked_add_signed(Duration::days(1))?
            .and_hms_opt(0, 0, 0)?,
    )?;
    Some((start, end))
}

/// Le jour local d'hier (jour par defaut du travail de 00:05).
#[must_use]
pub fn yesterday(tz: Tz, now: DateTime<Utc>) -> NaiveDate {
    now.with_timezone(&tz).date_naive() - Duration::days(1)
}

/// (minimum, maximum) d'une cle sur la fenetre ; rien si l'appareil n'existe pas (Reference : avertissement et zero).
async fn range<D, L>(
    devices: &D,
    log: &L,
    name: &str,
    key: &str,
    (from, to): (DateTime<Utc>, DateTime<Utc>),
) -> RepoResult<(Option<f64>, Option<f64>)>
where
    D: DeviceRepo + Sync,
    L: StateLogRepo + Sync,
{
    if let Some(d) = devices.find_by_name(name).await? {
        log.min_max(d.id, key, from, to).await
    } else {
        tracing::warn!(device = %name, "energy balance: device not found");
        Ok((None, None))
    }
}

/// Calcule le bilan du jour sans l'enregistrer.
///
/// # Errors
/// Erreur de lecture de la base.
pub async fn compute<D, L>(
    devices: &D,
    log: &L,
    names: &EnergyDevices,
    tz: Tz,
    day: NaiveDate,
) -> RepoResult<EnergyDay>
where
    D: DeviceRepo + Sync,
    L: StateLogRepo + Sync,
{
    let Some((from, to)) = day_window(tz, day) else {
        return Ok(EnergyDay {
            day,
            linky_hp_wh: 0,
            linky_hc_wh: 0,
            solar_wh: 0,
            pac_wh: 0,
        });
    };
    let hp = range(devices, log, &names.linky, "HCHP", (from, to)).await?;
    let hc = range(devices, log, &names.linky, "HCHC", (from, to)).await?;
    let qdy = range(devices, log, &names.solar, "qdy", (from, to)).await?;
    let ea = range(devices, log, &names.pac, "energy_a", (from, to)).await?;
    let eb = range(devices, log, &names.pac, "energy_b", (from, to)).await?;
    Ok(EnergyDay {
        day,
        linky_hp_wh: cumulative_delta(hp.0, hp.1),
        linky_hc_wh: cumulative_delta(hc.0, hc.1),
        solar_wh: solar_wh_from_qdy(qdy.1),
        pac_wh: cumulative_delta(ea.0, ea.1) + cumulative_delta(eb.0, eb.1),
    })
}

/// Calcule et enregistre (cree ou remplace) le bilan du jour.
///
/// # Errors
/// Erreur de lecture ou d'ecriture de la base.
pub async fn compute_and_store<D, L, E>(
    devices: &D,
    log: &L,
    energy: &E,
    names: &EnergyDevices,
    tz: Tz,
    day: NaiveDate,
) -> RepoResult<EnergyDay>
where
    D: DeviceRepo + Sync,
    L: StateLogRepo + Sync,
    E: EnergyRepo + Sync,
{
    let result = compute(devices, log, names, tz, day).await?;
    energy.upsert(&result).await?;
    Ok(result)
}
