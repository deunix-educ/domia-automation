// SPDX-License-Identifier: AGPL-3.0-or-later
//! Energy summary of a day (R4 lot 7, ADR 0036): `compute_daily_energy` of the reference implementation.
//!
//! The day is the **local day** (from local midnight to the next local midnight, 23 or 25 h on clock-change days), the
//! Linky meters (`HCHP`, `HCHC`) and the heat pump's (`energy_a` + `energy_b`) are cumulative (maximum - minimum of the day),
//! solar production is the maximum of `qdy` (kWh reset every day) converted to Wh. A device that is not found counts zero.

use chrono::{DateTime, Duration, NaiveDate, Utc};
use chrono_tz::Tz;
use dominet_domain::energy::{EnergyDay, cumulative_delta, solar_wh_from_qdy};
use dominet_domain::repo::{DeviceRepo, EnergyRepo, RepoResult, StateLogRepo};

use crate::next::to_utc;

/// Names (`friendly_name`) of the summary's devices: `ENERGY_LINKY_DEVICE`, `ENERGY_SOLAR_DEVICE`, `ENERGY_PAC_DEVICE`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnergyDevices {
    pub linky: String,
    pub solar: String,
    pub pac: String,
}

impl Default for EnergyDevices {
    /// The production's names.
    fn default() -> Self {
        Self {
            linky: "consommation".into(),
            solar: "prod".into(),
            pac: "PAC-CE".into(),
        }
    }
}

/// State keys to **track** (history): those of the daily summary (`HCHP`, `HCHC`, `qdy`, `energy_a`,
/// `energy_b`) and the instantaneous powers of the Grafana "Energy" dashboard (`PAPP`, `pac`, `power_ab`).
/// An empty key is ignored.
#[must_use]
pub fn required_keys(devices: &EnergyDevices) -> [(String, [&'static str; 3]); 3] {
    [
        (devices.linky.clone(), ["HCHP", "HCHC", "PAPP"]),
        (devices.solar.clone(), ["qdy", "pac", ""]),
        (devices.pac.clone(), ["energy_a", "energy_b", "power_ab"]),
    ]
}

/// Interval `[start, end[` (UTC) of the local day `day`.
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

/// Yesterday's local day (default day of the 00:05 job).
#[must_use]
pub fn yesterday(tz: Tz, now: DateTime<Utc>) -> NaiveDate {
    now.with_timezone(&tz).date_naive() - Duration::days(1)
}

/// (minimum, maximum) of a key over the window; nothing if the device does not exist (reference: warning and zero).
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

/// Computes the day's summary without saving it.
///
/// # Errors
/// Database read error.
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

/// Computes and saves (creates or replaces) the day's summary.
///
/// # Errors
/// Database read or write error.
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
