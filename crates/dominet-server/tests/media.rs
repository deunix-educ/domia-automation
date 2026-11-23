// SPDX-License-Identifier: AGPL-3.0-or-later
//! Device images: upload by form (administrators), serving with `ETag` (logged-in accounts).

mod common;

use axum::http::{Method, StatusCode, header};
use common::{Client, PASSWORD, add_user, state};
use dominet_domain::device::DeviceDraft;
use dominet_domain::repo::DeviceRepo;
use dominet_domain::user::Role;
use image::{DynamicImage, ImageBuffer, ImageFormat, Rgb};
use sqlx::PgPool;

fn png(w: u32, h: u32) -> Vec<u8> {
    let img = ImageBuffer::from_pixel(w, h, Rgb([10u8, 120, 200]));
    let mut out = Vec::new();
    DynamicImage::ImageRgb8(img)
        .write_to(&mut std::io::Cursor::new(&mut out), ImageFormat::Png)
        .unwrap();
    out
}

/// `multipart/form-data` body of a file field.
fn multipart(field: &str, file: &[u8]) -> (String, Vec<u8>) {
    let boundary = "----dominet-test-boundary";
    let mut body = format!(
        "--{boundary}\r\nContent-Disposition: form-data; name=\"{field}\"; filename=\"x.bin\"\r\nContent-Type: application/octet-stream\r\n\r\n"
    )
    .into_bytes();
    body.extend_from_slice(file);
    body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
    (format!("multipart/form-data; boundary={boundary}"), body)
}

async fn logged(state: &dominet_server::AppState, name: &str, role: Role) -> Client {
    add_user(state, name, role).await;
    let mut c = Client::new(state);
    assert_eq!(c.login(name, PASSWORD).await.status, StatusCode::OK);
    c
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn an_administrator_uploads_an_image_everyone_signed_in_can_see_it_with_etag(pool: PgPool) {
    let st = state(pool, &[]);
    let id = st
        .store
        .devices()
        .create(&DeviceDraft::new("Lampe"))
        .await
        .unwrap()
        .id
        .0;
    let mut admin = logged(&st, "alice", Role::Admin).await;
    let mut user = logged(&st, "bob", Role::User).await;
    let mut anon = Client::new(&st);
    let upload = format!("/api/devices/{id}/image");
    let (ct, body) = multipart("image", &png(1600, 1200));

    // Rights: neither anonymous nor plain account can upload; without an image, 404.
    for who in [&mut anon, &mut user] {
        let (status, _, _) = who
            .send_bytes(Method::POST, &upload, &ct, body.clone(), &[])
            .await;
        assert!(
            matches!(status, StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN),
            "{status}"
        );
    }
    assert_eq!(
        user.get(&format!("/media/devices/{id}")).await.status,
        StatusCode::NOT_FOUND
    );

    // Upload: redirect to the device's page; the image is reduced and re-encoded as JPEG.
    let (status, headers, _) = admin
        .send_bytes(Method::POST, &upload, &ct, body, &[])
        .await;
    assert_eq!(status, StatusCode::SEE_OTHER);
    assert_eq!(
        headers[header::LOCATION],
        format!("/automation/devices/{id}?image=saved").as_str()
    );
    let (status, headers, bytes) = user
        .send_bytes(
            Method::GET,
            &format!("/media/devices/{id}"),
            "text/plain",
            vec![],
            &[],
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(headers[header::CONTENT_TYPE], "image/jpeg");
    assert_eq!(headers[header::CACHE_CONTROL], "private, no-cache");
    let shown = image::load_from_memory(&bytes).unwrap();
    assert_eq!((shown.width(), shown.height()), (400, 300));

    // Revalidation: 304 without a body as long as the image does not change.
    let etag = headers[header::ETAG].to_str().unwrap().to_string();
    let (status, _, bytes) = user
        .send_bytes(
            Method::GET,
            &format!("/media/devices/{id}"),
            "text/plain",
            vec![],
            &[("if-none-match", &etag)],
        )
        .await;
    assert_eq!((status, bytes.len()), (StatusCode::NOT_MODIFIED, 0));
    assert_eq!(
        anon.get(&format!("/media/devices/{id}")).await.status,
        StatusCode::UNAUTHORIZED
    );

    // A file that is not an image is refused and replaces nothing.
    let (ct, junk) = multipart("image", b"<script>alert(1)</script>");
    let (status, headers, _) = admin
        .send_bytes(Method::POST, &upload, &ct, junk, &[])
        .await;
    assert_eq!(status, StatusCode::SEE_OTHER);
    assert!(
        headers[header::LOCATION]
            .to_str()
            .unwrap()
            .ends_with("image=unsupported")
    );
    let (ct, other) = multipart("autre", b"x");
    let (_, headers, _) = admin
        .send_bytes(Method::POST, &upload, &ct, other, &[])
        .await;
    assert!(
        headers[header::LOCATION]
            .to_str()
            .unwrap()
            .ends_with("image=missing")
    );
    assert_eq!(
        user.get(&format!("/media/devices/{id}")).await.status,
        StatusCode::OK
    );

    // Unknown device; deletion (administrator only).
    let (ct, body) = multipart("image", &png(10, 10));
    let (status, _, _) = admin
        .send_bytes(
            Method::POST,
            &format!("/api/devices/{}/image", id + 1000),
            &ct,
            body,
            &[],
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let delete = format!("/api/devices/{id}/image/delete");
    let (status, _, _) = user
        .send_bytes(Method::POST, &delete, "text/plain", vec![], &[])
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, headers, _) = admin
        .send_bytes(Method::POST, &delete, "text/plain", vec![], &[])
        .await;
    assert_eq!(status, StatusCode::SEE_OTHER);
    assert!(
        headers[header::LOCATION]
            .to_str()
            .unwrap()
            .ends_with("image=deleted")
    );
    assert_eq!(
        user.get(&format!("/media/devices/{id}")).await.status,
        StatusCode::NOT_FOUND
    );
}
