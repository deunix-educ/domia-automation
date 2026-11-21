// SPDX-License-Identifier: AGPL-3.0-or-later
//! Detection de mouvement (R6 lot 8, ADR 0050) : **le meme algorithme que reference** (`detect_motion_camera`, `OpenCV`) en Rust pur.
//!
//! Deux images (la premiere et la derniere d'une fenetre) sont passees en niveaux de gris, floutees (noyau gaussien 21 x 21, ecart type
//! derive de la taille comme `GaussianBlur(sigma = 0)`), soustraites en valeur absolue, seuillees (> 25), **ouvertes** (erosion puis
//! dilatation par un carre de 5 x 5), et le rapport de pixels restants au total est compare a `MOTION_THRESHOLD`. Les details d'`OpenCV`
//! sont reproduits : conversion en gris en entiers (`0,299 R + 0,587 G + 0,114 B` a 14 bits), bords du flou en reflexion
//! (`BORDER_REFLECT_101`), bords de l'ouverture neutres. Le flou d'`OpenCV` sur images 8 bits utilise un calcul en virgule fixe que l'on
//! ne reproduit pas bit a bit : l'ecart au rapport d'`OpenCV` est mesure par un test de parite (tolerance dans l'ADR).

use std::sync::Arc;

use image::GenericImageView;

/// Taille du noyau de flou (Reference : `(21, 21)`).
pub const BLUR_KERNEL: usize = 21;
/// Seuil de difference de niveau de gris (Reference : 25).
pub const DIFF_THRESHOLD: u8 = 25;
/// Cote du carre d'ouverture (Reference : `np.ones((5, 5))`).
pub const OPEN_KERNEL: usize = 5;

#[derive(Debug, thiserror::Error)]
pub enum MotionError {
    #[error("image illisible : {0}")]
    Decode(String),
    #[error("les deux images n'ont pas la meme taille")]
    SizeMismatch,
}

/// Image en niveaux de gris, une ligne apres l'autre.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Gray {
    pub width: usize,
    pub height: usize,
    pub data: Vec<u8>,
}

/// Decode un JPEG et le passe en niveaux de gris comme `cv2.cvtColor(..., COLOR_BGR2GRAY)`.
///
/// # Errors
/// JPEG invalide.
pub fn decode_gray(jpeg: &[u8]) -> Result<Gray, MotionError> {
    let img = image::load_from_memory_with_format(jpeg, image::ImageFormat::Jpeg)
        .map_err(|e| MotionError::Decode(e.to_string()))?;
    let (w, h) = img.dimensions();
    let rgb = img.to_rgb8();
    Ok(gray_from_rgb(
        rgb.as_raw(),
        usize::try_from(w).unwrap_or(0),
        usize::try_from(h).unwrap_or(0),
    ))
}

/// Gris entier d'`OpenCV` : `(R * 4899 + G * 9617 + B * 1868 + 8192) >> 14`.
#[must_use]
pub fn gray_from_rgb(rgb: &[u8], width: usize, height: usize) -> Gray {
    let data = rgb
        .as_chunks::<3>()
        .0
        .iter()
        .take(width * height)
        .map(|p| {
            let v = u32::from(p[0]) * 4899 + u32::from(p[1]) * 9617 + u32::from(p[2]) * 1868 + 8192;
            u8::try_from(v >> 14).unwrap_or(u8::MAX)
        })
        .collect();
    Gray {
        width,
        height,
        data,
    }
}

/// Noyau gaussien normalise de `ksize` points, ecart type de `GaussianBlur(sigma = 0)` : `0,3 * ((k - 1) / 2 - 1) + 0,8`.
#[must_use]
pub fn gaussian_kernel(ksize: usize) -> Vec<f32> {
    #[allow(clippy::cast_precision_loss, reason = "ksize <= 21")]
    let (k, sigma) = {
        let k = ksize as f32;
        (k, 0.3 * ((k - 1.0) * 0.5 - 1.0) + 0.8)
    };
    let centre = (k - 1.0) / 2.0;
    #[allow(clippy::cast_precision_loss, reason = "indices <= 21")]
    let mut kernel: Vec<f32> = (0..ksize)
        .map(|i| {
            let d = i as f32 - centre;
            (-(d * d) / (2.0 * sigma * sigma)).exp()
        })
        .collect();
    let sum: f32 = kernel.iter().sum();
    for v in &mut kernel {
        *v /= sum;
    }
    kernel
}

/// Reflexion sans repetition du bord (`BORDER_REFLECT_101`) : `-1 -> 1`, `n -> n - 2`.
fn reflect101(i: isize, n: usize) -> usize {
    if n == 1 {
        return 0;
    }
    let n = isize::try_from(n).unwrap_or(isize::MAX);
    let period = 2 * (n - 1);
    let mut i = i.rem_euclid(period);
    if i >= n {
        i = period - i;
    }
    usize::try_from(i).unwrap_or(0)
}

/// Flou gaussien separable, bords en reflexion, arrondi a l'entier.
#[must_use]
pub fn gaussian_blur(src: &Gray, ksize: usize) -> Gray {
    let (w, h) = (src.width, src.height);
    if w == 0 || h == 0 {
        return src.clone();
    }
    let kernel = gaussian_kernel(ksize);
    let half = isize::try_from(ksize / 2).unwrap_or(0);
    // Horizontal.
    let mut tmp = vec![0.0f32; w * h];
    for y in 0..h {
        let row = &src.data[y * w..(y + 1) * w];
        for x in 0..w {
            let mut acc = 0.0;
            for (k, kv) in kernel.iter().enumerate() {
                let dx = isize::try_from(k).unwrap_or(0) - half;
                let sx = reflect101(isize::try_from(x).unwrap_or(0) + dx, w);
                acc += kv * f32::from(row[sx]);
            }
            tmp[y * w + x] = acc;
        }
    }
    // Vertical.
    let mut out = vec![0u8; w * h];
    for y in 0..h {
        for x in 0..w {
            let mut acc = 0.0;
            for (k, kv) in kernel.iter().enumerate() {
                let dy = isize::try_from(k).unwrap_or(0) - half;
                let sy = reflect101(isize::try_from(y).unwrap_or(0) + dy, h);
                acc += kv * tmp[sy * w + x];
            }
            #[allow(
                clippy::cast_possible_truncation,
                clippy::cast_sign_loss,
                reason = "borne a 0..=255"
            )]
            {
                out[y * w + x] = (acc + 0.5).clamp(0.0, 255.0) as u8;
            }
        }
    }
    Gray {
        width: w,
        height: h,
        data: out,
    }
}

/// Erosion (`min`) ou dilatation (`max`) par un carre de `side` points, separable ; les points hors de l'image sont ignores
/// (valeur par defaut d'`OpenCV` pour la morphologie).
fn morph(src: &[u8], w: usize, h: usize, side: usize, erode: bool) -> Vec<u8> {
    let half = side / 2;
    let pick = |a: u8, b: u8| if erode { a.min(b) } else { a.max(b) };
    let neutral = if erode { u8::MAX } else { 0 };
    let mut tmp = vec![neutral; w * h];
    for y in 0..h {
        for x in 0..w {
            let lo = x.saturating_sub(half);
            let hi = (x + half).min(w - 1);
            tmp[y * w + x] = src[y * w + lo..=y * w + hi]
                .iter()
                .fold(neutral, |m, &v| pick(m, v));
        }
    }
    let mut out = vec![neutral; w * h];
    for y in 0..h {
        let lo = y.saturating_sub(half);
        let hi = (y + half).min(h - 1);
        for x in 0..w {
            out[y * w + x] = (lo..=hi).fold(neutral, |m, yy| pick(m, tmp[yy * w + x]));
        }
    }
    out
}

/// Rapport de pixels en mouvement entre deux images deja floutees : seuil, ouverture, comptage.
///
/// # Errors
/// Les deux images n'ont pas la meme taille.
pub fn motion_ratio_of_blurred(a: &Gray, b: &Gray) -> Result<f64, MotionError> {
    if (a.width, a.height) != (b.width, b.height) {
        return Err(MotionError::SizeMismatch);
    }
    let (w, h) = (a.width, a.height);
    if w == 0 || h == 0 {
        return Ok(0.0);
    }
    let binary: Vec<u8> = a
        .data
        .iter()
        .zip(&b.data)
        .map(|(&p, &q)| {
            if p.abs_diff(q) > DIFF_THRESHOLD {
                255
            } else {
                0
            }
        })
        .collect();
    let eroded = morph(&binary, w, h, OPEN_KERNEL, true);
    let opened = morph(&eroded, w, h, OPEN_KERNEL, false);
    let moving = opened.iter().filter(|&&v| v > 0).count();
    #[allow(clippy::cast_precision_loss, reason = "comptage de pixels, < 2^52")]
    Ok(moving as f64 / opened.len() as f64)
}

/// Le rapport de mouvement entre deux JPEG (premiere et derniere image d'une fenetre), comme `detect_motion_camera`.
///
/// # Errors
/// JPEG illisible ou de tailles differentes.
pub fn motion_ratio(first: &[u8], last: &[u8]) -> Result<f64, MotionError> {
    let a = gaussian_blur(&decode_gray(first)?, BLUR_KERNEL);
    let b = gaussian_blur(&decode_gray(last)?, BLUR_KERNEL);
    motion_ratio_of_blurred(&a, &b)
}

/// Une image decodee partageable entre taches (la fenetre de la camera garde des JPEG, pas des images brutes).
pub type SharedJpeg = Arc<Vec<u8>>;

#[cfg(test)]
mod tests {
    use super::*;

    fn flat(w: usize, h: usize, v: u8) -> Gray {
        Gray {
            width: w,
            height: h,
            data: vec![v; w * h],
        }
    }

    #[test]
    fn gray_uses_the_integer_weights_of_opencv() {
        let g = gray_from_rgb(&[255, 0, 0, 0, 255, 0, 0, 0, 255, 255, 255, 255], 4, 1);
        assert_eq!(g.data, [76, 150, 29, 255]);
    }

    #[test]
    fn the_kernel_is_normalised_symmetric_and_has_the_opencv_sigma() {
        let k = gaussian_kernel(21);
        assert!((k.iter().sum::<f32>() - 1.0).abs() < 1e-5);
        assert!((k[0] - k[20]).abs() < 1e-7 && k[10] > k[9]);
        // sigma = 3,5 : le centre vaut 1 / (sigma * sqrt(2 pi)) ~ 0,114.
        assert!((k[10] - 0.1140).abs() < 0.001, "{}", k[10]);
    }

    #[test]
    fn reflect101_does_not_repeat_the_edge() {
        assert_eq!(reflect101(-1, 5), 1);
        assert_eq!(reflect101(-2, 5), 2);
        assert_eq!(reflect101(5, 5), 3);
        assert_eq!(reflect101(6, 5), 2);
        assert_eq!(reflect101(3, 1), 0);
    }

    #[test]
    fn blurring_a_flat_image_changes_nothing_and_spreads_a_point() {
        assert_eq!(gaussian_blur(&flat(30, 30, 90), 21).data, vec![90; 900]);
        let mut spike = flat(41, 41, 0);
        spike.data[20 * 41 + 20] = 255;
        let blurred = gaussian_blur(&spike, 21);
        // 255 * 0,114 * 0,114 ~ 3,3.
        assert!((2..=4).contains(&blurred.data[20 * 41 + 20]));
        assert_eq!(blurred.data[0], 0);
    }

    #[test]
    fn identical_images_have_no_motion_and_a_big_change_has_a_lot() {
        let a = flat(64, 48, 100);
        assert!(motion_ratio_of_blurred(&a, &a).unwrap().abs() < f64::EPSILON);
        let mut b = flat(64, 48, 100);
        for y in 10..40 {
            for x in 10..50 {
                b.data[y * 64 + x] = 200;
            }
        }
        let r = motion_ratio_of_blurred(&a, &b).unwrap();
        assert!((r - (30.0 * 40.0) / (64.0 * 48.0)).abs() < 1e-9, "{r}");
    }

    #[test]
    fn the_opening_removes_specks_smaller_than_the_kernel() {
        let a = flat(40, 40, 0);
        let mut b = flat(40, 40, 0);
        // Un point isole et une ligne fine : disparaissent a l'ouverture 5 x 5.
        b.data[10 * 40 + 10] = 255;
        for x in 0..40 {
            b.data[30 * 40 + x] = 255;
        }
        assert!(motion_ratio_of_blurred(&a, &b).unwrap().abs() < f64::EPSILON);
        // Un carre de 6 x 6 survit entierement.
        for y in 5..11 {
            for x in 20..26 {
                b.data[y * 40 + x] = 255;
            }
        }
        let r = motion_ratio_of_blurred(&a, &b).unwrap();
        assert!((r - 36.0 / 1600.0).abs() < 1e-9, "{r}");
    }

    #[test]
    fn the_threshold_is_strict_and_sizes_must_match() {
        let a = flat(20, 20, 100);
        assert!(
            motion_ratio_of_blurred(&a, &flat(20, 20, 125))
                .unwrap()
                .abs()
                < f64::EPSILON
        );
        assert!(motion_ratio_of_blurred(&a, &flat(20, 20, 126)).unwrap() > 0.99);
        assert!(matches!(
            motion_ratio_of_blurred(&a, &flat(10, 20, 0)),
            Err(MotionError::SizeMismatch)
        ));
        assert!(matches!(
            decode_gray(b"pas un jpeg"),
            Err(MotionError::Decode(_))
        ));
    }
}
