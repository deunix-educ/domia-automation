// SPDX-License-Identifier: AGPL-3.0-or-later
//! Icones Lucide rendues cote serveur (feature `ssr`).
//!
//! Le jeu complet (Lucide v1.21.0, licence ISC, `data/lucide.tsv`, genere par `tools/gen-lucide.py`) reste dans le
//! binaire serveur : le navigateur ne recoit que le SVG des icones affichees (champ `icon_svg` des DTO), donc
//! **aucun octet de plus dans le WASM**, et l'administrateur peut choisir n'importe quelle icone Lucide.
//!
//! Securite : le SVG renvoye vient uniquement de la table embarquee (balises de dessin et attributs de geometrie,
//! verifies a la generation) ; un nom saisi par un utilisateur sert de cle de recherche, jamais de contenu.

use std::collections::HashMap;
use std::fmt::Write;
use std::sync::OnceLock;

static DATA: &str = include_str!("../data/lucide.tsv");
/// Icone de repli pour un nom inconnu (le premier de ces noms present dans le jeu est utilise).
const FALLBACKS: [&str; 3] = ["circle-help", "circle-question-mark", "circle"];

fn table() -> &'static HashMap<&'static str, &'static str> {
    static TABLE: OnceLock<HashMap<&'static str, &'static str>> = OnceLock::new();
    TABLE.get_or_init(|| {
        let mut map = HashMap::new();
        for line in DATA
            .lines()
            .filter(|l| !l.is_empty() && !l.starts_with('#'))
        {
            if let Some((names, markup)) = line.split_once('\t') {
                for name in names.split(' ') {
                    map.insert(name, markup);
                }
            }
        }
        map
    })
}

/// Un nom d'icone valide : minuscules, chiffres et tirets, 1 a 64 caracteres.
#[must_use]
pub fn is_valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

/// L'icone existe-t-elle dans le jeu ?
#[must_use]
pub fn exists(name: &str) -> bool {
    is_valid_name(name) && table().contains_key(name)
}

/// Tous les noms d'icones (alias compris), tries : pour un futur selecteur d'icones.
#[must_use]
pub fn names() -> Vec<&'static str> {
    let mut v: Vec<_> = table().keys().copied().collect();
    v.sort_unstable();
    v
}

fn inner(name: &str) -> &'static str {
    if is_valid_name(name)
        && let Some(m) = table().get(name)
    {
        return m;
    }
    FALLBACK_MARKUP.get_or_init(|| {
        FALLBACKS
            .iter()
            .find_map(|n| table().get(n).copied())
            .unwrap_or("")
    })
}

static FALLBACK_MARKUP: OnceLock<&'static str> = OnceLock::new();

/// SVG complet (`size` pixels, couleur du texte courant) ; icone de repli si le nom est inconnu.
#[must_use]
pub fn svg(name: &str, size: u32) -> String {
    let mut out = String::with_capacity(320);
    let _ = write!(
        out,
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="{size}" height="{size}" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" class="vc-icon" aria-hidden="true" focusable="false">{}</svg>"#,
        inner(name)
    );
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Icones utilisees par le projet reference (templates et JS) et par la navigation par defaut.
    const USED: [&str; 30] = [
        "zap",
        "layout-grid",
        "cpu",
        "check",
        "bar-chart-2",
        "x",
        "wind",
        "wifi-off",
        "video-off",
        "video",
        "square",
        "refresh-cw",
        "play",
        "maximize",
        "lock-open",
        "lock",
        "loader",
        "layers",
        "hard-drive",
        "gauge",
        "droplets",
        "cloud-rain",
        "chevron-down",
        "camera",
        "settings-2",
        "activity",
        "book-open",
        "wifi-sync",
        "smartphone",
        "mail",
    ];

    #[test]
    fn every_icon_used_by_the_reference_project_exists() {
        for name in USED {
            assert!(exists(name), "{name}");
        }
    }

    #[test]
    fn the_default_navigation_icons_exist() {
        for item in dominet_domain::registry::default_navbar() {
            assert!(exists(&item.icon_name), "{}", item.icon_name);
        }
    }

    #[test]
    fn the_set_is_complete() {
        assert!(names().len() >= 1900, "{}", names().len());
        assert!(
            names().windows(2).all(|w| w[0] < w[1]),
            "tries, sans doublon"
        );
    }

    #[test]
    fn svg_is_a_well_formed_sized_wrapper() {
        let s = svg("video", 16);
        assert!(s.starts_with("<svg ") && s.ends_with("</svg>"), "{s}");
        assert!(s.contains(r#"width="16" height="16""#) && s.contains(r#"viewBox="0 0 24 24""#));
        assert!(s.contains(r#"stroke="currentColor""#) && s.contains(r#"class="vc-icon""#));
        assert!(svg("video", 28).contains(r#"width="28" height="28""#));
    }

    #[test]
    fn unknown_or_hostile_names_fall_back_to_a_safe_icon() {
        let fallback = svg("circle-help", 16);
        for bad in [
            "n-existe-pas",
            "",
            "Video",
            "../../etc/passwd",
            "<script>alert(1)</script>",
            "video onload=x",
            &"a".repeat(65),
        ] {
            assert_eq!(svg(bad, 16), fallback, "{bad:?}");
            assert!(
                !svg(bad, 16).contains(bad.trim()) || bad.is_empty(),
                "le nom ne doit jamais se retrouver dans le SVG"
            );
        }
        assert!(
            fallback.contains("<path") || fallback.contains("<circle"),
            "le repli n'est pas vide"
        );
    }

    #[test]
    fn markup_only_uses_drawing_tags_and_no_active_content() {
        let allowed = [
            "path", "line", "polyline", "polygon", "circle", "rect", "ellipse",
        ];
        for name in names() {
            let s = svg(name, 16);
            for forbidden in [
                "<script",
                "javascript:",
                "onload",
                "onerror",
                "href=",
                "style=",
                "<foreignObject",
                "<use",
            ] {
                assert!(!s.contains(forbidden), "{name}: {forbidden}");
            }
            let inner = s.split_once('>').unwrap().1.trim_end_matches("</svg>");
            for tag in inner.split('<').filter(|t| !t.is_empty()) {
                let t = tag.split([' ', '/', '>']).next().unwrap();
                assert!(allowed.contains(&t), "{name}: balise {t}");
                assert!(
                    tag.trim_end().ends_with("/>"),
                    "{name}: balise non fermee : {tag}"
                );
            }
        }
    }
}
