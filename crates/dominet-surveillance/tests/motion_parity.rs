// SPDX-License-Identifier: AGPL-3.0-or-later
//! Parite du mouvement avec `OpenCV` (R6 lot 8, ADR 0050) : les rapports de mouvement de paires d'images synthetiques, calcules par
//! la detection de mouvement de reference (`OpenCV` 5), sont retrouves par l'implementation en Rust pur.

use std::path::PathBuf;

use dominet_surveillance::motion::motion_ratio;

/// Ecart absolu tolere sur le rapport (mesure : 0,0002 au plus) (fraction de l'image). Les JPEG sont decodes par deux decodeurs differents et le flou 8 bits
/// d'`OpenCV` est en virgule fixe : quelques pixels de bord de zone peuvent basculer.
const TOLERANCE: f64 = 0.0005;

fn dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures/motion")
}

#[test]
fn rust_ratios_match_the_opencv_reference() {
    let reference: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(dir().join("reference.json")).unwrap())
            .unwrap();
    let first = std::fs::read(dir().join("first.jpg")).unwrap();
    let ratios = reference["ratios"].as_object().unwrap();
    assert!(ratios.len() >= 8);
    for (name, expected) in ratios {
        let expected = expected.as_f64().unwrap();
        let other = std::fs::read(dir().join(format!("{name}.jpg"))).unwrap();
        let got = motion_ratio(&first, &other).unwrap();
        println!("{name:<18} opencv {expected:.5}  rust {got:.5}");
        assert!(
            (got - expected).abs() <= TOLERANCE,
            "{name} : OpenCV {expected}, Rust {got}"
        );
    }
}

#[test]
fn the_decision_against_the_reference_default_threshold_is_the_same() {
    // Meme decision que reference avec MOTION_THRESHOLD = 0,01 pour chaque cas.
    let reference: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(dir().join("reference.json")).unwrap())
            .unwrap();
    let first = std::fs::read(dir().join("first.jpg")).unwrap();
    for (name, expected) in reference["ratios"].as_object().unwrap() {
        let other = std::fs::read(dir().join(format!("{name}.jpg"))).unwrap();
        let got = motion_ratio(&first, &other).unwrap();
        assert_eq!(
            got > 0.01,
            expected.as_f64().unwrap() > 0.01,
            "{name} : decision differente"
        );
    }
}
