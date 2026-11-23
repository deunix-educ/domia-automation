// SPDX-License-Identifier: AGPL-3.0-or-later
//! Motion parity with `OpenCV` (R6 lot 8, ADR 0050): the motion ratios of pairs of synthetic images, computed by
//! the reference motion detection (`OpenCV` 5), are found again by the pure-Rust implementation.

use std::path::PathBuf;

use dominet_surveillance::motion::motion_ratio;

/// Absolute difference tolerated on the ratio (measured: 0.0002 at most) (fraction of the image). The JPEGs are decoded by two different decoders and `OpenCV`'s 8-bit blur
/// is fixed-point: a few region-edge pixels may flip.
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
    // Same decision as the reference with MOTION_THRESHOLD = 0.01 for each case.
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
