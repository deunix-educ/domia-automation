// SPDX-License-Identifier: AGPL-3.0-or-later
//! Object detection (R6 lot 8, ADR 0009 and 0050): `YOLOv8n` through ONNX Runtime (`ort`), same contract as the reference's `OnnxBackend`.
//!
//! Input 640 x 640 NCHW `float32` RGB / 255 (direct resizing, without keeping the aspect ratio), output `[1, 84, 8400]`
//! (`cx, cy, w, h` then 80 class scores), score >= 0.10 before non-maximum suppression (`IoU` 0.45, regardless of class).
//! The user's confidence threshold and the class filter are applied by the caller, as in the reference.
//!
//! A **pool of sessions with a single compute thread each** (ADR 0009: the most efficient per core); `detect_*` blocks, call it from
//! `spawn_blocking`.

use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Mutex, OnceLock};

use image::{ImageEncoder, RgbImage};
use ndarray::Array4;
use ort::session::Session;
use ort::value::TensorRef;

const SIZE: usize = 640;
const SIZE_PX: u32 = 640;
/// Minimum score before non-maximum suppression.
pub const SCORE_THRESHOLD: f32 = 0.10;
/// `IoU` beyond which a box is suppressed.
pub const NMS_IOU: f32 = 0.45;

/// The 80 COCO classes, in `YOLOv8n`'s training order.
pub const COCO_CLASSES: [&str; 80] = [
    "person",
    "bicycle",
    "car",
    "motorcycle",
    "airplane",
    "bus",
    "train",
    "truck",
    "boat",
    "traffic light",
    "fire hydrant",
    "stop sign",
    "parking meter",
    "bench",
    "bird",
    "cat",
    "dog",
    "horse",
    "sheep",
    "cow",
    "elephant",
    "bear",
    "zebra",
    "giraffe",
    "backpack",
    "umbrella",
    "handbag",
    "tie",
    "suitcase",
    "frisbee",
    "skis",
    "snowboard",
    "sports ball",
    "kite",
    "baseball bat",
    "baseball glove",
    "skateboard",
    "surfboard",
    "tennis racket",
    "bottle",
    "wine glass",
    "cup",
    "fork",
    "knife",
    "spoon",
    "bowl",
    "banana",
    "apple",
    "sandwich",
    "orange",
    "broccoli",
    "carrot",
    "hot dog",
    "pizza",
    "donut",
    "cake",
    "chair",
    "couch",
    "potted plant",
    "bed",
    "dining table",
    "toilet",
    "tv",
    "laptop",
    "mouse",
    "remote",
    "keyboard",
    "cell phone",
    "microwave",
    "oven",
    "toaster",
    "sink",
    "refrigerator",
    "book",
    "clock",
    "vase",
    "scissors",
    "teddy bear",
    "hair drier",
    "toothbrush",
];

#[derive(Debug, thiserror::Error)]
pub enum YoloError {
    #[error("modele illisible ({path}) : {message}")]
    Model { path: String, message: String },
    #[error("image illisible : {0}")]
    Image(String),
    #[error("inference impossible : {0}")]
    Inference(String),
}

/// A recognised object: COCO class, score, box `[x1, y1, x2, y2]` in pixels of the original image.
#[derive(Debug, Clone, PartialEq)]
pub struct Det {
    pub class: &'static str,
    pub confidence: f32,
    pub bbox: [f32; 4],
}

/// Bilinear resizing with pixel centres (`OpenCV`'s `INTER_LINEAR`, without anti-aliasing) directly into a normalised
/// NCHW tensor.
#[must_use]
#[allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "indices et dimensions d'image << 2^24 ; positions flottantes bornees a l'image"
)]
pub fn preprocess(img: &RgbImage) -> Array4<f32> {
    let (w, h) = (img.width() as usize, img.height() as usize);
    let raw = img.as_raw();
    let (sx, sy) = (w as f32 / SIZE as f32, h as f32 / SIZE as f32);
    let mut out = Array4::<f32>::zeros((1, 3, SIZE, SIZE));
    let xs: Vec<(usize, usize, f32)> = (0..SIZE)
        .map(|x| {
            let fx = ((x as f32 + 0.5) * sx - 0.5).max(0.0);
            let x0 = (fx as usize).min(w - 1);
            (x0, (x0 + 1).min(w - 1), fx - x0 as f32)
        })
        .collect();
    for y in 0..SIZE {
        let fy = ((y as f32 + 0.5) * sy - 0.5).max(0.0);
        let y0 = (fy as usize).min(h - 1);
        let (y1, wy) = ((y0 + 1).min(h - 1), fy - y0 as f32);
        for (x, &(x0, x1, wx)) in xs.iter().enumerate() {
            for c in 0..3 {
                let p = |yy: usize, xx: usize| f32::from(raw[(yy * w + xx) * 3 + c]);
                let top = p(y0, x0) * (1.0 - wx) + p(y0, x1) * wx;
                let bot = p(y1, x0) * (1.0 - wx) + p(y1, x1) * wx;
                out[[0, c, y, x]] = (top * (1.0 - wy) + bot * wy) / 255.0;
            }
        }
    }
    out
}

/// Intersection over union of two boxes `[x1, y1, x2, y2]`.
#[must_use]
pub fn iou(a: &[f32; 4], b: &[f32; 4]) -> f32 {
    let (ix1, iy1, ix2, iy2) = (
        a[0].max(b[0]),
        a[1].max(b[1]),
        a[2].min(b[2]),
        a[3].min(b[3]),
    );
    let inter = (ix2 - ix1).max(0.0) * (iy2 - iy1).max(0.0);
    let (aa, ab) = ((a[2] - a[0]) * (a[3] - a[1]), (b[2] - b[0]) * (b[3] - b[1]));
    inter / (aa + ab - inter).max(1e-9)
}

/// Network output `[1, 84, 8400]` -> detections (score >= 0.10, `NMS` `IoU` 0.45), boxes scaled back to the `orig_w` x `orig_h` image.
#[must_use]
#[allow(clippy::cast_precision_loss, reason = "dimensions d'image << 2^24")]
pub fn postprocess(out: &ndarray::ArrayViewD<'_, f32>, orig_w: u32, orig_h: u32) -> Vec<Det> {
    if out.ndim() != 3 || out.shape()[1] < 84 {
        return vec![];
    }
    let n = out.shape()[2];
    let (sx, sy) = (orig_w as f32 / SIZE as f32, orig_h as f32 / SIZE as f32);
    let mut candidates: Vec<Det> = vec![];
    for i in 0..n {
        let (mut best, mut class) = (0.0f32, 0usize);
        for c in 0..80 {
            let s = out[[0, 4 + c, i]];
            if s > best {
                best = s;
                class = c;
            }
        }
        if best < SCORE_THRESHOLD {
            continue;
        }
        let (cx, cy, bw, bh) = (
            out[[0, 0, i]],
            out[[0, 1, i]],
            out[[0, 2, i]],
            out[[0, 3, i]],
        );
        candidates.push(Det {
            class: COCO_CLASSES[class],
            confidence: best,
            bbox: [
                (cx - bw / 2.0) * sx,
                (cy - bh / 2.0) * sy,
                (cx + bw / 2.0) * sx,
                (cy + bh / 2.0) * sy,
            ],
        });
    }
    candidates.sort_by(|a, b| b.confidence.total_cmp(&a.confidence));
    let mut kept: Vec<Det> = vec![];
    for d in candidates {
        if kept.iter().all(|k| iou(&k.bbox, &d.bbox) <= NMS_IOU) {
            kept.push(d);
        }
    }
    kept
}

/// Draws the boxes on a copy of the JPEG (green 2-point lines, like the reference; **without text**: no embedded font) and
/// returns a new JPEG.
///
/// # Errors
/// Unreadable JPEG.
pub fn annotate(jpeg: &[u8], detections: &[Det]) -> Result<Vec<u8>, YoloError> {
    let mut img = image::load_from_memory_with_format(jpeg, image::ImageFormat::Jpeg)
        .map_err(|e| YoloError::Image(e.to_string()))?
        .to_rgb8();
    let (w, h) = (img.width(), img.height());
    for d in detections {
        #[allow(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            clippy::cast_precision_loss,
            reason = "boites bornees a l'image juste avant ; dimensions << 2^24"
        )]
        let [x1, y1, x2, y2] = d.bbox.map(|v| v.round().clamp(0.0, w.max(h) as f32) as u32);
        let (x1, x2) = (x1.min(w - 1), x2.min(w - 1));
        let (y1, y2) = (y1.min(h - 1), y2.min(h - 1));
        for t in 0..2 {
            for x in x1..=x2 {
                for y in [y1 + t, y2.saturating_sub(t)] {
                    img.put_pixel(x, y.min(h - 1), image::Rgb([0, 255, 0]));
                }
            }
            for y in y1..=y2 {
                for x in [x1 + t, x2.saturating_sub(t)] {
                    img.put_pixel(x.min(w - 1), y, image::Rgb([0, 255, 0]));
                }
            }
        }
    }
    let mut out = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, 85)
        .write_image(img.as_raw(), w, h, image::ExtendedColorType::Rgb8)
        .map_err(|e| YoloError::Image(e.to_string()))?;
    Ok(out)
}

/// Pool of ONNX Runtime sessions with one compute thread each.
pub struct YoloDetector {
    sessions: Vec<Mutex<Session>>,
    input_name: String,
    next: AtomicUsize,
}

impl std::fmt::Debug for YoloDetector {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("YoloDetector")
            .field("workers", &self.sessions.len())
            .finish_non_exhaustive()
    }
}

/// ONNX Runtime admits one environment per process: the first load creates it, the following ones (other detectors, parallel tests)
/// reuse it. Without this, two simultaneous loads failed ("Only one instance of `LoggingManager`").
static ENVIRONMENT: OnceLock<Result<ort::environment::Environment, String>> = OnceLock::new();

impl YoloDetector {
    /// Loads the model into `workers` sessions (at least one) and warms them up with an empty pass.
    ///
    /// # Errors
    /// Model missing or invalid.
    pub fn load(model: &Path, workers: usize) -> Result<Self, YoloError> {
        let fail = |e: &dyn std::fmt::Display| YoloError::Model {
            path: model.display().to_string(),
            message: e.to_string(),
        };
        let env = ENVIRONMENT
            .get_or_init(|| {
                ort::environment::init()
                    .with_name("dominet")
                    .build()
                    .map_err(|e| e.to_string())
            })
            .as_ref()
            .map_err(|e| fail(e))?;
        let mut sessions: Vec<Mutex<Session>> = Vec::new();
        for _ in 0..workers.max(1) {
            let builder = Session::builder(env).map_err(|e| fail(&e))?;
            let mut builder = builder.with_intra_threads(1).map_err(|e| fail(&e))?;
            let session: Session = builder.commit_from_file(model).map_err(|e| fail(&e))?;
            sessions.push(Mutex::new(session));
        }
        let input_name = sessions[0]
            .lock()
            .map_err(|_| fail(&"verrou empoisonne"))?
            .inputs()
            .first()
            .map(|i| i.name().to_string())
            .ok_or_else(|| fail(&"the model has no input"))?;
        let detector = Self {
            sessions,
            input_name,
            next: AtomicUsize::new(0),
        };
        detector.detect_rgb(&RgbImage::new(SIZE_PX, SIZE_PX))?;
        Ok(detector)
    }

    /// Number of sessions.
    #[must_use]
    pub fn workers(&self) -> usize {
        self.sessions.len()
    }

    /// Detects on a JPEG.
    ///
    /// # Errors
    /// Unreadable JPEG or inference failure.
    pub fn detect_jpeg(&self, jpeg: &[u8]) -> Result<Vec<Det>, YoloError> {
        let img = image::load_from_memory_with_format(jpeg, image::ImageFormat::Jpeg)
            .map_err(|e| YoloError::Image(e.to_string()))?
            .to_rgb8();
        self.detect_rgb(&img)
    }

    /// Detects on an RGB image.
    ///
    /// # Errors
    /// Inference failure.
    pub fn detect_rgb(&self, img: &RgbImage) -> Result<Vec<Det>, YoloError> {
        let input = preprocess(img);
        // A free session if possible, otherwise we wait for the next one in turn.
        let start = self.next.fetch_add(1, Ordering::Relaxed);
        let n = self.sessions.len();
        let mut guard = (0..n)
            .find_map(|k| self.sessions[(start + k) % n].try_lock().ok())
            .map_or_else(
                || {
                    self.sessions[start % n]
                        .lock()
                        .map_err(|_| YoloError::Inference("verrou empoisonne".into()))
                },
                Ok,
            )?;
        let fail = |e: &dyn std::fmt::Display| YoloError::Inference(e.to_string());
        let outputs = guard
            .run(ort::inputs![self.input_name.as_str() => TensorRef::from_array_view(&input).map_err(|e| fail(&e))?])
            .map_err(|e| fail(&e))?;
        let array = outputs[0]
            .try_extract_array::<f32>()
            .map_err(|e| fail(&e))?;
        Ok(postprocess(&array, img.width(), img.height()))
    }
}

#[cfg(test)]
mod tests {
    use ndarray::Array3;

    use super::*;

    fn output(rows: &[(f32, f32, f32, f32, usize, f32)]) -> Array3<f32> {
        let mut a = Array3::<f32>::zeros((1, 84, 8400));
        for (i, &(cx, cy, w, h, class, score)) in rows.iter().enumerate() {
            a[[0, 0, i]] = cx;
            a[[0, 1, i]] = cy;
            a[[0, 2, i]] = w;
            a[[0, 3, i]] = h;
            a[[0, 4 + class, i]] = score;
        }
        a
    }

    #[test]
    fn boxes_are_scaled_to_the_original_image_and_weak_scores_dropped() {
        let out = output(&[
            (320.0, 320.0, 100.0, 200.0, 0, 0.9),
            (50.0, 50.0, 10.0, 10.0, 2, 0.05),
        ]);
        let found = postprocess(&out.view().into_dyn(), 1280, 360);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].class, "person");
        // x: 640 -> 1280 (x2), y: 640 -> 360 (x0.5625).
        let b = found[0].bbox;
        assert!((b[0] - 540.0).abs() < 0.01, "{b:?}");
        assert!(
            (b[2] - b[0] - 200.0).abs() < 0.01 && (b[3] - b[1] - 112.5).abs() < 0.01,
            "{b:?}"
        );
    }

    #[test]
    fn overlapping_boxes_keep_the_best_whatever_their_class() {
        let out = output(&[
            (100.0, 100.0, 80.0, 80.0, 0, 0.6),
            (104.0, 102.0, 80.0, 80.0, 16, 0.9),
            (400.0, 400.0, 50.0, 50.0, 2, 0.4),
        ]);
        let found = postprocess(&out.view().into_dyn(), 640, 640);
        assert_eq!(
            found.iter().map(|d| d.class).collect::<Vec<_>>(),
            ["dog", "car"]
        );
    }

    #[test]
    fn iou_is_zero_apart_and_one_for_equal_boxes() {
        let a = [0.0, 0.0, 10.0, 10.0];
        assert!(iou(&a, &[20.0, 20.0, 30.0, 30.0]).abs() < f32::EPSILON);
        assert!((iou(&a, &a) - 1.0).abs() < 1e-6);
        assert!((iou(&a, &[5.0, 0.0, 15.0, 10.0]) - 1.0 / 3.0).abs() < 1e-6);
    }

    #[test]
    fn a_malformed_output_gives_nothing() {
        let a = Array3::<f32>::zeros((1, 10, 5));
        assert_eq!(postprocess(&a.view().into_dyn(), 640, 640).len(), 0);
    }

    #[test]
    fn preprocess_normalises_and_keeps_the_channel_order() {
        let mut img = RgbImage::new(4, 4);
        for p in img.pixels_mut() {
            *p = image::Rgb([255, 0, 51]);
        }
        let t = preprocess(&img);
        assert!((t[[0, 0, 7, 9]] - 1.0).abs() < 1e-6);
        assert!(t[[0, 1, 7, 9]].abs() < 1e-6);
        assert!((t[[0, 2, 639, 639]] - 0.2).abs() < 1e-6);
    }

    #[test]
    fn annotate_draws_green_and_stays_a_jpeg() {
        let img = RgbImage::from_pixel(64, 48, image::Rgb([0, 0, 0]));
        let mut jpeg = Vec::new();
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut jpeg, 90)
            .write_image(img.as_raw(), 64, 48, image::ExtendedColorType::Rgb8)
            .unwrap();
        let out = annotate(
            &jpeg,
            &[Det {
                class: "person",
                confidence: 0.9,
                bbox: [10.0, 10.0, 40.0, 30.0],
            }],
        )
        .unwrap();
        let back = image::load_from_memory(&out).unwrap().to_rgb8();
        assert!(
            back.get_pixel(25, 10).0[1] > 150,
            "{:?}",
            back.get_pixel(25, 10)
        );
        assert!(back.get_pixel(25, 20).0[1] < 60, "l'interieur reste sombre");
        assert!(annotate(b"pas un jpeg", &[]).is_err());
    }
}
