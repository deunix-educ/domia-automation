// SPDX-License-Identifier: AGPL-3.0-or-later
//! Device images (R3 lot 5, ADR 0029): upload by form (administrators) and serving (logged-in accounts).
//!
//! The image is decoded, **reduced to fit in 400 x 300** and re-encoded as JPEG: what is stored is always a sound image,
//! of bounded size, whatever file was sent (a file that is not an image, or is too big, is refused).
//! The images are in the database (`device_image`): nothing to back up separately.

use std::io::Cursor;

use axum::Router;
use axum::extract::{DefaultBodyLimit, Multipart, Path, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Redirect, Response};
use axum::routing::{get, post};
use dominet_domain::device::DeviceId;
use dominet_domain::repo::{DeviceImageRepo, DeviceRepo};
use image::codecs::jpeg::JpegEncoder;
use image::imageops::FilterType;
use image::{ImageFormat, ImageReader, Limits, Rgb, RgbImage};

use crate::AppState;
use crate::auth::extract::{CurrentUser, RequireAdmin};
use crate::error::{AppError, AppResult};

/// Maximum size of the uploaded file.
pub const MAX_UPLOAD_BYTES: usize = 5 * 1024 * 1024;
/// Maximum dimensions of the stored image.
pub const MAX_WIDTH: u32 = 400;
pub const MAX_HEIGHT: u32 = 300;
/// Maximum dimensions accepted on input (a 60000 x 60000 pixel image fits in a few KB compressed).
const MAX_INPUT_SIDE: u32 = 8000;
const JPEG_QUALITY: u8 = 85;

/// Why an image is refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageProblem {
    /// Neither JPEG, PNG, WebP nor GIF (or unreadable).
    Unsupported,
    TooLarge,
}

/// Decodes, reduces and re-encodes an uploaded file.
///
/// # Errors
/// [`ImageProblem`] if it is not a readable JPEG, PNG, WebP or GIF image, or if it is too big.
pub fn process_upload(bytes: &[u8]) -> Result<Vec<u8>, ImageProblem> {
    if bytes.len() > MAX_UPLOAD_BYTES {
        return Err(ImageProblem::TooLarge);
    }
    let mut reader = ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|_| ImageProblem::Unsupported)?;
    if !matches!(
        reader.format(),
        Some(ImageFormat::Jpeg | ImageFormat::Png | ImageFormat::WebP | ImageFormat::Gif)
    ) {
        return Err(ImageProblem::Unsupported);
    }
    let mut limits = Limits::default();
    limits.max_image_width = Some(MAX_INPUT_SIDE);
    limits.max_image_height = Some(MAX_INPUT_SIDE);
    limits.max_alloc = Some(256 * 1024 * 1024);
    reader.limits(limits);
    let decoded = reader.decode().map_err(|e| match e {
        image::ImageError::Limits(_) => ImageProblem::TooLarge,
        _ => ImageProblem::Unsupported,
    })?;
    let fitted = if decoded.width() > MAX_WIDTH || decoded.height() > MAX_HEIGHT {
        decoded.resize(MAX_WIDTH, MAX_HEIGHT, FilterType::Triangle)
    } else {
        decoded
    };
    // Transparency: composited onto white (JPEG has no alpha channel).
    let rgba = fitted.to_rgba8();
    let mut rgb = RgbImage::new(rgba.width(), rgba.height());
    for (x, y, p) in rgba.enumerate_pixels() {
        let a = u32::from(p[3]);
        let blend =
            |c: u8| u8::try_from((u32::from(c) * a + 255 * (255 - a) + 127) / 255).unwrap_or(255);
        rgb.put_pixel(x, y, Rgb([blend(p[0]), blend(p[1]), blend(p[2])]));
    }
    let mut out = Vec::new();
    JpegEncoder::new_with_quality(&mut out, JPEG_QUALITY)
        .encode_image(&rgb)
        .map_err(|_| ImageProblem::Unsupported)?;
    Ok(out)
}

/// Write routes, under `/api`.
pub fn api_routes() -> Router<AppState> {
    Router::new()
        .route("/devices/{id}/image", post(upload))
        .route("/devices/{id}/image/delete", post(remove))
        .layer(DefaultBodyLimit::max(MAX_UPLOAD_BYTES + 64 * 1024))
}

/// Read route: `/media/devices/{id}`.
pub fn media_routes() -> Router<AppState> {
    Router::new().route("/media/devices/{id}", get(serve))
}

fn back(id: i64, outcome: &str) -> Redirect {
    Redirect::to(&format!("/automation/devices/{id}?image={outcome}"))
}

async fn upload(
    State(state): State<AppState>,
    RequireAdmin(admin): RequireAdmin,
    Path(id): Path<i64>,
    mut form: Multipart,
) -> AppResult<Redirect> {
    if state.store.devices().get(DeviceId(id)).await?.is_none() {
        return Err(AppError::NotFound);
    }
    let mut file = None;
    loop {
        match form.next_field().await {
            Ok(Some(field)) if field.name() == Some("image") => match field.bytes().await {
                Ok(bytes) => {
                    file = Some(bytes);
                    break;
                }
                Err(_) => return Ok(back(id, "toolarge")),
            },
            Ok(Some(_)) => {}
            Ok(None) => break,
            Err(_) => return Ok(back(id, "toolarge")),
        }
    }
    let Some(bytes) = file.filter(|b| !b.is_empty()) else {
        return Ok(back(id, "missing"));
    };
    // Decoding is pure computation: off the request thread.
    let processed = tokio::task::spawn_blocking(move || process_upload(&bytes))
        .await
        .map_err(AppError::internal)?;
    match processed {
        Ok(jpeg) => {
            state
                .store
                .device_images()
                .put(DeviceId(id), "image/jpeg", &jpeg)
                .await?;
            tracing::info!(
                admin_id = admin.id.0,
                device_id = id,
                bytes = jpeg.len(),
                "device image saved"
            );
            Ok(back(id, "saved"))
        }
        Err(ImageProblem::TooLarge) => Ok(back(id, "toolarge")),
        Err(ImageProblem::Unsupported) => Ok(back(id, "unsupported")),
    }
}

async fn remove(
    State(state): State<AppState>,
    RequireAdmin(admin): RequireAdmin,
    Path(id): Path<i64>,
) -> AppResult<Redirect> {
    if state.store.devices().get(DeviceId(id)).await?.is_none() {
        return Err(AppError::NotFound);
    }
    state.store.device_images().delete(DeviceId(id)).await?;
    tracing::info!(
        admin_id = admin.id.0,
        device_id = id,
        "device image deleted"
    );
    Ok(back(id, "deleted"))
}

async fn serve(
    State(state): State<AppState>,
    CurrentUser(_): CurrentUser,
    Path(id): Path<i64>,
    headers: HeaderMap,
) -> AppResult<Response> {
    let image = state
        .store
        .device_images()
        .get(DeviceId(id))
        .await?
        .ok_or(AppError::NotFound)?;
    let etag = format!(
        "\"{}-{}\"",
        image.updated_at.timestamp_micros(),
        image.data.len()
    );
    let mut response_headers = HeaderMap::new();
    response_headers.insert(
        header::ETAG,
        HeaderValue::from_str(&etag).map_err(AppError::internal)?,
    );
    // `private`: the image is not public; `no-cache`: revalidated by ETag at each display (304 without a body).
    response_headers.insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("private, no-cache"),
    );
    if headers
        .get(header::IF_NONE_MATCH)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v == etag)
    {
        return Ok((StatusCode::NOT_MODIFIED, response_headers).into_response());
    }
    response_headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_str(&image.content_type).map_err(AppError::internal)?,
    );
    Ok((response_headers, image.data).into_response())
}

#[cfg(test)]
mod tests {
    use image::{DynamicImage, ImageBuffer, Rgba};

    use super::*;

    fn png(w: u32, h: u32, alpha: u8) -> Vec<u8> {
        let img = ImageBuffer::from_pixel(w, h, Rgba([200u8, 30, 30, alpha]));
        let mut out = Vec::new();
        DynamicImage::ImageRgba8(img)
            .write_to(&mut Cursor::new(&mut out), ImageFormat::Png)
            .unwrap();
        out
    }

    fn size(jpeg: &[u8]) -> (u32, u32) {
        let img = image::load_from_memory_with_format(jpeg, ImageFormat::Jpeg).unwrap();
        (img.width(), img.height())
    }

    #[test]
    fn a_large_image_is_reduced_to_fit_400_by_300_keeping_its_proportions() {
        let out = process_upload(&png(2000, 1000, 255)).unwrap();
        assert_eq!(size(&out), (400, 200));
        let tall = process_upload(&png(300, 1200, 255)).unwrap();
        assert_eq!(size(&tall), (75, 300));
    }

    #[test]
    fn a_small_image_keeps_its_size_and_the_output_is_always_a_jpeg() {
        let out = process_upload(&png(120, 80, 255)).unwrap();
        assert_eq!(size(&out), (120, 80));
        assert_eq!(&out[..2], &[0xFF, 0xD8], "signature JPEG");
    }

    #[test]
    fn transparency_is_flattened_on_white() {
        let out = process_upload(&png(10, 10, 0)).unwrap();
        let img = image::load_from_memory(&out).unwrap().to_rgb8();
        let p = img.get_pixel(5, 5);
        assert!(p.0.iter().all(|c| *c > 245), "{p:?}");
    }

    #[test]
    fn anything_that_is_not_a_readable_image_is_refused() {
        assert_eq!(
            process_upload(b"<script>alert(1)</script>"),
            Err(ImageProblem::Unsupported)
        );
        assert_eq!(process_upload(b""), Err(ImageProblem::Unsupported));
        // PNG signature, then bytes that do not make an image.
        assert_eq!(
            process_upload(&[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, 1, 2, 3]),
            Err(ImageProblem::Unsupported)
        );
        let mut truncated = png(50, 50, 255);
        truncated.truncate(40);
        assert_eq!(process_upload(&truncated), Err(ImageProblem::Unsupported));
    }

    #[test]
    fn oversized_files_and_dimensions_are_refused() {
        assert_eq!(
            process_upload(&vec![0u8; MAX_UPLOAD_BYTES + 1]),
            Err(ImageProblem::TooLarge)
        );
        // 9000 x 10 pixels: small file, but exceeds the accepted dimension.
        assert_eq!(
            process_upload(&png(9000, 10, 255)),
            Err(ImageProblem::TooLarge)
        );
    }
}
