// SPDX-License-Identifier: AGPL-3.0-or-later
//! YOLO parity with the reference (R6 lot 8, ADR 0050): the detections of `OnnxBackend` (ONNX Runtime + `OpenCV`) on
//! reference images are found again by `YoloDetector` (same class, box `IoU` > 0.9, score difference < 0.02: criteria of spike 5).
//!
//! The model is not in the repository: `make fetch-model`. Without it, these tests say so and check nothing.

#![allow(
    clippy::cast_possible_truncation,
    reason = "valeurs JSON de reference converties en f32 comme le detecteur"
)]

use std::path::PathBuf;

use dominet_surveillance::yolo::{YoloDetector, iou};

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn model() -> Option<PathBuf> {
    let path = std::env::var_os("YOLO_MODEL_PATH")
        .map_or_else(|| root().join("models/yolov8n.onnx"), PathBuf::from);
    if path.exists() {
        Some(path)
    } else {
        eprintln!(
            "NOTE : modele YOLO absent ({}), `make fetch-model` : parite non verifiee ici",
            path.display()
        );
        None
    }
}

fn fixtures() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures/yolo")
}

#[test]
fn every_reference_detection_is_found_and_nothing_else_above_the_threshold() {
    let Some(model) = model() else { return };
    let detector = YoloDetector::load(&model, 1).unwrap();
    let reference: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(fixtures().join("reference.json")).unwrap())
            .unwrap();
    let images = reference["images"].as_object().unwrap();
    assert!(images.len() >= 4);
    let (mut matched, mut expected_total) = (0, 0);
    for (name, expected) in images {
        let got = detector
            .detect_jpeg(&std::fs::read(fixtures().join(name)).unwrap())
            .unwrap();
        let expected = expected.as_array().unwrap();
        for e in expected {
            let class = e["class"].as_str().unwrap();
            let conf = e["confidence"].as_f64().unwrap() as f32;
            let bbox: Vec<f32> = e["bbox"]
                .as_array()
                .unwrap()
                .iter()
                .map(|v| v.as_f64().unwrap() as f32)
                .collect();
            let bbox = [bbox[0], bbox[1], bbox[2], bbox[3]];
            let found = got.iter().any(|d| {
                d.class == class && iou(&d.bbox, &bbox) > 0.9 && (d.confidence - conf).abs() < 0.02
            });
            assert!(found, "{name} : {class} {conf} {bbox:?} absent de {got:?}");
            matched += 1;
        }
        expected_total += expected.len();
        println!(
            "{name:<14} reference {:>2} | Rust {:>2}",
            expected.len(),
            got.len()
        );
        // Nothing extra, give or take a few borderline detections (score close to the 0.10 threshold).
        assert!(
            got.len() <= expected.len() + 1,
            "{name} : {} detections de plus que reference",
            got.len() - expected.len()
        );
    }
    println!("PARITE : {matched}/{expected_total}");
    assert_eq!(matched, expected_total);
}

#[test]
fn an_image_without_objects_gives_nothing_and_workers_share_the_load() {
    let Some(model) = model() else { return };
    let detector = std::sync::Arc::new(YoloDetector::load(&model, 2).unwrap());
    assert_eq!(detector.workers(), 2);
    let empty = std::fs::read(fixtures().join("empty.jpg")).unwrap();
    let bus = std::fs::read(fixtures().join("bus.jpg")).unwrap();
    assert_eq!(detector.detect_jpeg(&empty).unwrap().len(), 0);
    // Several simultaneous calls: all succeed, with the same result.
    let handles: Vec<_> = (0..6)
        .map(|_| {
            let (d, b) = (detector.clone(), bus.clone());
            std::thread::spawn(move || d.detect_jpeg(&b).unwrap().len())
        })
        .collect();
    let counts: Vec<usize> = handles.into_iter().map(|h| h.join().unwrap()).collect();
    assert!(
        counts.iter().all(|&c| c == counts[0] && c >= 5),
        "{counts:?}"
    );
    assert!(detector.detect_jpeg(b"pas un jpeg").is_err());
}

#[test]
fn a_missing_model_is_a_clear_error() {
    let err = YoloDetector::load(std::path::Path::new("/inexistant/modele.onnx"), 1).unwrap_err();
    assert!(err.to_string().contains("modele.onnx"), "{err}");
}
