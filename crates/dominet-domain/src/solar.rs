// SPDX-License-Identifier: AGPL-3.0-or-later
//! Evenements solaires (R4 lot 4, ADR 0033) : lever, coucher, midi solaire, aubes et crepuscules, calcules sans dependance.
//!
//! Position du soleil par les series de Meeus (precision ~0,01°), equation du temps, angle horaire. Memes conventions que
//! les evenements solaires de reference / `ephem` (pression 0, altitude 0) : lever et coucher quand le **bord superieur** touche l'horizon
//! a −34′ (centre a environ −0,83°), crepuscules quand le **centre** atteint −6°, −12° ou −18°. Compare a `ephem` par
//! ecart maximal tolere 90 s (mesure sur 1 036 cas : 13 s, `tests/schedule_oracle.rs`), hors cercles polaires.

use chrono::{DateTime, TimeZone, Utc};

use crate::automation::SolarEvent;

const J2000: f64 = 2_451_545.0;
const UNIX_EPOCH_JD: f64 = 2_440_587.5;

fn rad(deg: f64) -> f64 {
    deg.to_radians()
}

/// (declinaison en degres, equation du temps en minutes, distance en UA) au jour julien `jd`.
#[allow(clippy::many_single_char_names)] // les symboles de Meeus (T, L0, M, e, C...) sont ceux de la reference
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

/// Altitude du centre du soleil (degres) a laquelle l'evenement se produit.
fn altitude(event: SolarEvent, distance: f64) -> f64 {
    // Bord superieur a −34′ : le centre est plus bas du demi-diametre apparent.
    let limb = -34.0 / 60.0 - 0.266_49 / distance;
    match event {
        SolarEvent::Sunrise | SolarEvent::Sunset => limb,
        SolarEvent::SolarNoon => 0.0,
        SolarEvent::DawnCivil | SolarEvent::DuskCivil => -6.0,
        SolarEvent::DawnNautical | SolarEvent::DuskNautical => -12.0,
        SolarEvent::DawnAstronomical | SolarEvent::DuskAstronomical => -18.0,
    }
}

/// Instant (jour julien) de l'evenement autour du midi solaire du jour UTC `day` (jours depuis 1970), ou `None` si le soleil
/// n'atteint pas cette altitude ce jour-la (nuit polaire, soleil de minuit).
fn event_jd(event: SolarEvent, lat: f64, lon: f64, day: i64) -> Option<f64> {
    #[allow(clippy::cast_precision_loss)]
    // un nombre de jours depuis 1970 tient largement dans un f64
    let midnight = UNIX_EPOCH_JD + day as f64;
    // Midi solaire : on itere car l'equation du temps depend de l'instant.
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
    #[allow(clippy::cast_possible_truncation)] // millisecondes depuis 1970 : bien dans un i64
    let millis = ((jd - UNIX_EPOCH_JD) * 86_400_000.0).round() as i64;
    Utc.timestamp_millis_opt(millis).single()
}

/// Premier instant **strictement posterieur** a `after` ou l'evenement se produit au lieu donne (latitude nord et longitude
/// est positives). `None` si le soleil n'atteint jamais cette altitude dans les trois jours qui suivent (cercle polaire en
/// plein ete ou plein hiver) : l'appelant reessaie plus tard.
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
        // les evenements solaires de reference (ephem) donne 18:00:01 UTC (valeur du fichier d'oracle).
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
        // Tromso (69,65 N) : ni lever autour du 21 decembre, ni coucher autour du 21 juin (dans les 3 jours qui suivent).
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
