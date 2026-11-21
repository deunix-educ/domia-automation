// SPDX-License-Identifier: AGPL-3.0-or-later
//! Bilan energetique journalier (R4, ADR 0031) : Linky (heures pleines et creuses), production solaire, pompe a chaleur.

use std::fmt::Write;

use chrono::NaiveDate;

/// Un jour, en Wh.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EnergyDay {
    pub day: NaiveDate,
    pub linky_hp_wh: i32,
    pub linky_hc_wh: i32,
    pub solar_wh: i32,
    pub pac_wh: i32,
}

/// Compteur cumulatif sur une journee (la reference) : `max - min` des valeurs numeriques, tronque vers zero, jamais
/// negatif ; zero sans aucune valeur.
#[must_use]
pub fn cumulative_delta(min: Option<f64>, max: Option<f64>) -> i32 {
    match (min, max) {
        #[allow(clippy::cast_possible_truncation)]
        // comme `int(...)` de la reference ; tronque vers zero
        (Some(lo), Some(hi)) => (hi - lo).max(0.0) as i32,
        _ => 0,
    }
}

/// Production solaire du jour (maximum du jour, converti en Wh et tronque) : le maximum de `qdy` (kWh, remis a zero chaque jour)
/// converti en Wh, tronque vers zero.
#[must_use]
pub fn solar_wh_from_qdy(max_kwh: Option<f64>) -> i32 {
    #[allow(clippy::cast_possible_truncation)] // comme `int(...)` de la reference
    max_kwh.map_or(0, |hi| (hi * 1000.0) as i32)
}

impl EnergyDay {
    #[must_use]
    pub fn linky_total_wh(&self) -> i32 {
        self.linky_hp_wh + self.linky_hc_wh
    }

    /// Production - (consommation + PAC) : positif = surplus injecte (comme la reference).
    #[must_use]
    pub fn net_wh(&self) -> i32 {
        self.solar_wh - self.linky_total_wh() - self.pac_wh
    }
}

// ------------------------------------------------------------------ graphique SVG calcule cote serveur (aucune bibliotheque JS)

const W: f64 = 760.0;
const H: f64 = 300.0;
const LEFT: f64 = 52.0;
const RIGHT: f64 = 12.0;
const TOP: f64 = 14.0;
const BOTTOM: f64 = 44.0;
const SOLAR: &str = "#22c55e";
const LINKY: &str = "#f87171";
const PAC: &str = "#fb923c";
const NET: &str = "#60a5fa";

/// Pas « rond » (1, 2, 5 x 10^n) pour couvrir `span` en environ cinq graduations.
fn nice_step(span: f64) -> f64 {
    let raw = (span / 5.0).max(1.0);
    let magnitude = 10f64.powf(raw.log10().floor());
    let unit = raw / magnitude;
    let factor = if unit <= 1.0 {
        1.0
    } else if unit <= 2.0 {
        2.0
    } else if unit <= 5.0 {
        5.0
    } else {
        10.0
    };
    factor * magnitude
}

/// Graduation d'axe : `1500` -> `1.5k`, `500` -> `500`.
fn axis_label(v: f64) -> String {
    if v.abs() >= 1000.0 {
        let k = v / 1000.0;
        if (k - k.round()).abs() < 1e-9 {
            format!("{k:.0}k")
        } else {
            format!("{k:.1}k")
        }
    } else {
        format!("{v:.0}")
    }
}

/// Histogramme du bilan journalier (Wh) : solaire, Linky et PAC en barres, bilan net en courbe, comme la page de l'implementation de reference
/// (Chart.js) mais en SVG pur, sans script ni attribut `style` (CSP). Vide s'il n'y a aucun jour. `days` du plus ancien au plus recent.
#[must_use]
#[allow(clippy::too_many_lines)] // generateur de SVG : une longue suite d'ecritures, plus lisible d'un seul tenant
pub fn chart_svg(days: &[EnergyDay]) -> String {
    if days.is_empty() {
        return String::new();
    }
    let series = |d: &EnergyDay| {
        [
            f64::from(d.solar_wh),
            f64::from(d.linky_total_wh()),
            f64::from(d.pac_wh),
        ]
    };
    let nets: Vec<f64> = days.iter().map(|d| f64::from(d.net_wh())).collect();
    let high = days
        .iter()
        .flat_map(series)
        .chain(nets.iter().copied())
        .fold(0.0_f64, f64::max);
    let low = nets.iter().copied().fold(0.0_f64, f64::min);
    let step = nice_step(high - low);
    let top = (high / step).ceil() * step;
    let bottom = (low / step).floor() * step;
    let top = if top <= bottom { bottom + step } else { top };
    let (plot_w, plot_h) = (W - LEFT - RIGHT, H - TOP - BOTTOM);
    let y = |v: f64| TOP + (top - v) / (top - bottom) * plot_h;
    #[allow(clippy::cast_precision_loss)] // quelques centaines de jours au plus
    let n = days.len() as f64;
    let slot = plot_w / n;
    let bar = (slot * 0.8 / 3.0).max(1.0);
    let mut out = String::new();
    let _ = write!(
        out,
        r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {W} {H}" width="100%" role="img" id="energy-chart" font-family="sans-serif" font-size="10">"#
    );
    // Graduations horizontales et axe Y.
    let mut tick = bottom;
    while tick <= top + step * 1e-9 {
        let ty = y(tick);
        let _ = write!(
            out,
            r##"<line x1="{LEFT}" y1="{ty:.1}" x2="{:.1}" y2="{ty:.1}" stroke="#ffffff" stroke-opacity="{}" stroke-width="1"/><text x="{:.1}" y="{:.1}" fill="#9ca3af" text-anchor="end">{}</text>"##,
            W - RIGHT,
            if tick == 0.0 { "0.35" } else { "0.08" },
            LEFT - 6.0,
            ty + 3.0,
            axis_label(tick)
        );
        tick += step;
    }
    // Barres : une tranche par jour, trois barres cote a cote.
    let skip = (days.len() / 10 + 1).max(1);
    for (i, d) in days.iter().enumerate() {
        #[allow(clippy::cast_precision_loss)]
        let x0 = LEFT + slot * i as f64 + slot * 0.1;
        for (j, (value, color, name)) in [
            (f64::from(d.solar_wh), SOLAR, "Solar"),
            (f64::from(d.linky_total_wh()), LINKY, "Linky"),
            (f64::from(d.pac_wh), PAC, "PAC"),
        ]
        .into_iter()
        .enumerate()
        {
            let (top_y, base_y) = (y(value.max(0.0)), y(0.0));
            #[allow(clippy::cast_precision_loss)]
            let x = x0 + bar * j as f64;
            let _ = write!(
                out,
                r#"<rect x="{x:.1}" y="{top_y:.1}" width="{:.1}" height="{:.1}" fill="{color}" fill-opacity="0.75"><title>{} {name} : {value:.0} Wh</title></rect>"#,
                (bar - 1.0).max(0.5),
                (base_y - top_y).max(0.0),
                d.day
            );
        }
        if i % skip == 0 {
            let _ = write!(
                out,
                r##"<text x="{:.1}" y="{:.1}" fill="#6b7280" text-anchor="end" transform="rotate(-40 {:.1} {:.1})">{}</text>"##,
                x0 + slot * 0.4,
                H - BOTTOM + 14.0,
                x0 + slot * 0.4,
                H - BOTTOM + 14.0,
                d.day.format("%m-%d")
            );
        }
    }
    // Bilan net : courbe et points.
    let points: Vec<(f64, f64)> = nets
        .iter()
        .enumerate()
        .map(|(i, v)| {
            #[allow(clippy::cast_precision_loss)]
            let x = LEFT + slot * (i as f64 + 0.5);
            (x, y(*v))
        })
        .collect();
    let line: Vec<String> = points
        .iter()
        .map(|(x, yy)| format!("{x:.1},{yy:.1}"))
        .collect();
    let _ = write!(
        out,
        r#"<polyline points="{}" fill="none" stroke="{NET}" stroke-width="2"/>"#,
        line.join(" ")
    );
    for ((x, yy), (d, v)) in points.iter().zip(days.iter().zip(&nets)) {
        let _ = write!(
            out,
            r#"<circle cx="{x:.1}" cy="{yy:.1}" r="2.5" fill="{NET}"><title>{} bilan net : {v:+.0} Wh</title></circle>"#,
            d.day
        );
    }
    out.push_str("</svg>");
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn day(d: u32, solar: i32, hp: i32, hc: i32, pac: i32) -> EnergyDay {
        EnergyDay {
            day: NaiveDate::from_ymd_opt(2026, 6, d).unwrap(),
            linky_hp_wh: hp,
            linky_hc_wh: hc,
            solar_wh: solar,
            pac_wh: pac,
        }
    }

    #[test]
    fn the_chart_is_a_script_free_svg_with_three_bars_and_a_net_point_per_day() {
        assert_eq!(chart_svg(&[]), "");
        let days: Vec<EnergyDay> = (1..=12)
            .map(|d| day(d, 6000 + 100 * i32::try_from(d).unwrap(), 3000, 800, 1200))
            .collect();
        let svg = chart_svg(&days);
        assert!(svg.starts_with("<svg") && svg.ends_with("</svg>"));
        assert_eq!(svg.matches("<rect").count(), 36, "trois barres par jour");
        assert_eq!(
            svg.matches("<circle").count(),
            12,
            "un point de bilan net par jour"
        );
        assert_eq!(svg.matches("<polyline").count(), 1);
        for banned in ["style=", "<script", "onclick", "NaN", "inf"] {
            assert!(!svg.contains(banned), "{banned}");
        }
        assert!(
            svg.contains("Solar") && svg.contains("2026-06-12 bilan net : +"),
            "{}",
            &svg[..200]
        );
        // Un jour deficitaire descend sous zero : l'axe Y s'etend.
        let svg = chart_svg(&[day(1, 0, 5000, 2000, 1000), day(2, 8000, 100, 0, 0)]);
        assert!(svg.contains(">-"), "graduation negative");
        assert!(svg.contains("bilan net : -8000 Wh"));
    }

    #[test]
    fn axis_steps_are_round_and_labels_are_short() {
        for (span, step) in [
            (100.0, 20.0),
            (7500.0, 2000.0),
            (13000.0, 5000.0),
            (0.0, 1.0),
        ] {
            assert!((nice_step(span) - step).abs() < 1e-9, "{span}");
        }
        assert_eq!(axis_label(1500.0), "1.5k");
        assert_eq!(axis_label(2000.0), "2k");
        assert_eq!(axis_label(-500.0), "-500");
    }

    #[test]
    fn counters_truncate_like_the_reference_int() {
        assert_eq!(cumulative_delta(Some(100.0), Some(1234.9)), 1134);
        assert_eq!(cumulative_delta(Some(5.0), Some(5.0)), 0);
        assert_eq!(cumulative_delta(None, None), 0);
        assert_eq!(cumulative_delta(Some(10.0), Some(3.0)), 0, "jamais negatif");
        // `int(1.005 * 1000)` vaut 1004 en reference (1004.9999999999999) ; `int(4.35 * 1000)` vaut 4350.
        assert_eq!(solar_wh_from_qdy(Some(1.005)), 1004);
        assert_eq!(solar_wh_from_qdy(Some(4.35)), 4350);
        assert_eq!(solar_wh_from_qdy(None), 0);
    }

    #[test]
    fn the_balance_is_production_minus_consumption_and_heat_pump() {
        let d = EnergyDay {
            day: NaiveDate::from_ymd_opt(2026, 1, 1).unwrap(),
            linky_hp_wh: 4000,
            linky_hc_wh: 1500,
            solar_wh: 7000,
            pac_wh: 2000,
        };
        assert_eq!((d.linky_total_wh(), d.net_wh()), (5500, -500));
    }
}
