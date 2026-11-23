// SPDX-License-Identifier: AGPL-3.0-or-later
//! Solar events (R4 lot 4, ADR 0033): sunrise, sunset, solar noon, dawns and dusks, computed without dependencies.
//!
//! Position of the sun by Meeus series (accuracy ~0.01°), equation of time, hour angle. Same conventions as
//! the reference's solar events / `ephem` (pressure 0, altitude 0): sunrise and sunset when the **upper limb** touches the horizon
//! at −34′ (centre at about −0.83°), twilights when the **centre** reaches −6°, −12° or −18°. Compared with `ephem` by
//! maximum tolerated difference 90 s (measured on 1036 cases: 13 s, `tests/schedule_oracle.rs`), outside the polar circles.

use chrono::{DateTime, TimeZone, Utc};

use crate::automation::SolarEvent;

const J2000: f64 = 2_451_545.0;
const UNIX_EPOCH_JD: f64 = 2_440_587.5;

fn rad(deg: f64) -> f64 {
    deg.to_radians()
}

/// (declination in degrees, equation of time in minutes, distance in AU) at Julian day `jd`.
#[allow(clippy::many_single_char_names)] // Meeus's symbols (T, L0, M, e, C...) are those of the reference
fn sun(jd: f64) -> (f64, f64, f64) {
    let t = (jd - J2000) / 36525.0;
    let l0 = (280.466_46 + 36_000.769_83 * t + 0.000_303_2 * t * t).rem_euclid(360.0);
    let m = 357.529_11 + 35_999.050_29 * t - 0.000_153_7 * t * t;
    let e = 0.016_708_634 - 0.000_042_037 * t - 0.000_000_126_7 * t * t;
    let c = (1.914_602 - 0.004_817 * t - 0.000_014 * t * t) * rad(m).sin()
        + (0.019_993 - 0.000_101 * t) * rad(2.0 * m).sin()
        + 0.000_289 * rad(3.0 * m).sin();
    let true_long = l0 + c;
    let omega = 125.04 - 1934.136 * t;
    let lambda = true_long - 0.005_69 - 0.004_78 * rad(omega).sin();
    let eps0 =
        23.0 + (26.0 + (21.448 - t * (46.815 + t * (0.000_59 - t * 0.001_813))) / 60.0) / 60.0;
    let eps = eps0 + 0.002_56 * rad(omega).cos();
    let decl = (rad(eps).sin() * rad(lambda).sin()).asin().to_degrees();
    let y = rad(eps / 2.0).tan().powi(2);
    let eq = 4.0
        * (y * rad(2.0 * l0).sin() - 2.0 * e * rad(m).sin()
            + 4.0 * e * y * rad(m).sin() * rad(2.0 * l0).cos()
            - 0.5 * y * y * rad(4.0 * l0).sin()
            - 1.25 * e * e * rad(2.0 * m).sin())
        .to_degrees();
    let nu = m + c;
    let distance = 1.000_001_018 * (1.0 - e * e) / (1.0 + e * rad(nu).cos());
    (decl, eq, distance)
}

/// Altitude of the centre of the sun (degrees) at which the event occurs.
fn altitude(event: SolarEvent, distance: f64) -> f64 {
    // Upper limb at −34′: the centre is lower by the apparent half-diameter.
    let limb = -34.0 / 60.0 - 0.266_49 / distance;
    match event {
        SolarEvent::Sunrise | SolarEvent::Sunset => limb,
        SolarEvent::SolarNoon => 0.0,
        SolarEvent::DawnCivil | SolarEvent::DuskCivil => -6.0,
        SolarEvent::DawnNautical | SolarEvent::DuskNautical => -12.0,
        SolarEvent::DawnAstronomical | SolarEvent::DuskAstronomical => -18.0,
    }
}

/// Instant (Julian day) of the event around the solar noon of UTC day `day` (days since 1970), or `None` if the sun
/// does not reach this altitude that day (polar night, midnight sun).
fn event_jd(event: SolarEvent, lat: f64, lon: f64, day: i64) -> Option<f64> {
    #[allow(clippy::cast_precision_loss)]
    // a number of days since 1970 fits comfortably in an f64
    let midnight = UNIX_EPOCH_JD + day as f64;
    // Solar noon: we iterate because the equation of time depends on the instant.
    let mut transit = midnight + 0.5 - lon / 360.0;
    for _ in 0..3 {
        transit = midnight + 0.5 - lon / 360.0 - sun(transit).1 / 1440.0;
    }
    let rising = matches!(
        event,
        SolarEvent::Sunrise
            | SolarEvent::DawnCivil
            | SolarEvent::DawnNautical
            | SolarEvent::DawnAstronomical
    );
    if event == SolarEvent::SolarNoon {
        return Some(transit);
    }
    let mut t = transit;
    for _ in 0..4 {
        let (decl, _, distance) = sun(t);
        let h = altitude(event, distance);
        let cos_h =
            (rad(h).sin() - rad(lat).sin() * rad(decl).sin()) / (rad(lat).cos() * rad(decl).cos());
        if !(-1.0..=1.0).contains(&cos_h) {
            return None;
        }
        let hour_angle = cos_h.acos().to_degrees();
        let half_day = hour_angle / 360.0;
        t = if rising {
            transit - half_day
        } else {
            transit + half_day
        };
    }
    Some(t)
}

fn from_jd(jd: f64) -> Option<DateTime<Utc>> {
    #[allow(clippy::cast_possible_truncation)] // milliseconds since 1970: well within an i64
    let millis = ((jd - UNIX_EPOCH_JD) * 86_400_000.0).round() as i64;
    Utc.timestamp_millis_opt(millis).single()
}

/// First instant **strictly after** `after` at which the event occurs at the given place (north latitude and east longitude
/// positive). `None` if the sun never reaches this altitude in the following three days (polar circle in
/// midsummer or midwinter): the caller retries later.
#[must_use]
pub fn next_event(
    event: SolarEvent,
    lat: f64,
    lon: f64,
    after: DateTime<Utc>,
) -> Option<DateTime<Utc>> {
    let day = after.timestamp().div_euclid(86_400);
    (day - 1..=day + 3)
        .filter_map(|d| event_jd(event, lat, lon, d))
        .filter_map(from_jd)
        .filter(|t| *t > after)
        .min()
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;

    use super::*;

    fn utc(y: i32, mo: u32, d: u32, h: u32, mi: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(y, mo, d, h, mi, 0).unwrap()
    }

    #[test]
    fn marseille_sunset_on_the_28th_of_march_is_at_six_pm_utc() {
        // the reference solar events (ephem) give 18:00:01 UTC (value of the oracle file).
        let t = next_event(SolarEvent::Sunset, 43.3, 5.4, utc(2026, 3, 28, 12, 0)).unwrap();
        assert!(
            (t - utc(2026, 3, 28, 18, 0)).num_seconds().abs() <= 90,
            "{t}"
        );
    }

    #[test]
    fn events_follow_each_other_through_the_day_and_never_go_backwards() {
        let mut t = utc(2026, 6, 21, 0, 0);
        let mut seen = vec![];
        for event in [
            SolarEvent::DawnAstronomical,
            SolarEvent::DawnNautical,
            SolarEvent::DawnCivil,
            SolarEvent::Sunrise,
            SolarEvent::SolarNoon,
            SolarEvent::Sunset,
            SolarEvent::DuskCivil,
            SolarEvent::DuskNautical,
            SolarEvent::DuskAstronomical,
        ] {
            let next = next_event(event, 43.3, 5.4, t).unwrap();
            assert!(
                next >= t || event == SolarEvent::DawnAstronomical,
                "{event:?}"
            );
            seen.push(next);
            t = next;
        }
        assert!(seen.windows(2).all(|w| w[0] <= w[1]), "{seen:?}");
    }

    #[test]
    fn the_polar_night_and_the_midnight_sun_have_no_event() {
        // Tromsø (69.65 N): no sunrise around 21 December, no sunset around 21 June (within the following 3 days).
        assert_eq!(
            next_event(SolarEvent::Sunrise, 69.65, 18.96, utc(2026, 12, 20, 0, 0)),
            None
        );
        assert_eq!(
            next_event(SolarEvent::Sunset, 69.65, 18.96, utc(2026, 6, 20, 0, 0)),
            None
        );
    }
}
