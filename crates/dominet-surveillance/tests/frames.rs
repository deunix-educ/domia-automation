// SPDX-License-Identifier: AGPL-3.0-or-later
//! `FrameSource` avec un vrai `ffmpeg` et des sources `lavfi` (aucun reseau, aucune camera) : cadence, largeur, supervision, repli. Les tests
//! sont ignores (avec un message) sur une machine sans `ffmpeg`.

use std::time::Duration;

use dominet_surveillance::frames::{FfmpegFrameSource, FrameConfig, FrameSource, Hwaccel};

fn have_ffmpeg() -> bool {
    let ok = std::process::Command::new("ffmpeg")
        .arg("-version")
        .output()
        .is_ok_and(|o| o.status.success());
    if !ok {
        eprintln!("NOTE : ffmpeg absent, test non verifie ici");
    }
    ok
}

fn lavfi(size: &str, rate: u32, extra: &[&str]) -> Vec<String> {
    let mut v: Vec<String> = ["-re"].map(String::from).to_vec();
    v.extend(extra.iter().map(|s| (*s).to_string()));
    v.extend(
        [
            "-f",
            "lavfi",
            "-i",
            &format!("testsrc=size={size}:rate={rate}"),
        ]
        .map(String::from),
    );
    v
}

fn fast(input: Vec<String>) -> FrameConfig {
    let mut c = FrameConfig::new(input);
    c.hwaccel = Hwaccel::Off;
    c.backoff_min = Duration::from_millis(100);
    c.backoff_max = Duration::from_millis(300);
    c
}

#[tokio::test]
async fn frames_come_at_the_asked_rate_and_width_as_whole_jpegs() {
    if !have_ffmpeg() {
        return;
    }
    let mut cfg = fast(lavfi("1920x1080", 25, &[]));
    cfg.fps = 5.0;
    cfg.max_width = 640;
    let source = FfmpegFrameSource::spawn(cfg);
    let mut rx = source.subscribe();
    let mut frames = vec![];
    let start = std::time::Instant::now();
    while frames.len() < 6 && start.elapsed() < Duration::from_secs(15) {
        if let Ok(Ok(f)) = tokio::time::timeout(Duration::from_secs(5), rx.recv()).await {
            frames.push(f);
        }
    }
    assert!(frames.len() >= 6, "{} images", frames.len());
    for f in &frames {
        assert_eq!(
            (f.width, f.height),
            (640, 360),
            "ramene a 640 de large, hauteur proportionnelle"
        );
        assert!(f.jpeg.starts_with(&[0xFF, 0xD8]) && f.jpeg.ends_with(&[0xFF, 0xD9]));
    }
    assert!(
        frames.windows(2).all(|w| w[1].seq > w[0].seq),
        "numeros croissants"
    );
    // Cadence : 5 images par seconde, a une demi-seconde pres sur 6 images.
    let span = (frames[5].at - frames[0].at)
        .to_std()
        .unwrap()
        .as_secs_f64();
    assert!((0.7..=1.6).contains(&span), "5 intervalles en {span:.2} s");
    assert!(source.latest().is_some());
    assert!(source.status().running);
}

#[tokio::test]
async fn a_small_source_is_never_enlarged() {
    if !have_ffmpeg() {
        return;
    }
    let source = FfmpegFrameSource::spawn(fast(lavfi("320x180", 10, &[])));
    let frame = source
        .next_after(0, Duration::from_secs(15))
        .await
        .expect("une image");
    assert_eq!((frame.width, frame.height), (320, 180));
}

#[tokio::test]
async fn next_after_waits_for_a_fresher_frame_and_gives_up_after_the_timeout() {
    if !have_ffmpeg() {
        return;
    }
    let source = FfmpegFrameSource::spawn(fast(lavfi("320x180", 10, &[])));
    let first = source.next_after(0, Duration::from_secs(15)).await.unwrap();
    let next = source
        .next_after(first.seq, Duration::from_secs(5))
        .await
        .unwrap();
    assert!(next.seq > first.seq);
    // Une source qui ne donne rien : l'attente se termine.
    let mut dead = fast(vec!["-i".into(), "/nonexistent.mp4".into()]);
    dead.stall_after = Duration::from_secs(5);
    let silent = FfmpegFrameSource::spawn(dead);
    assert!(
        silent
            .next_after(0, Duration::from_millis(600))
            .await
            .is_none()
    );
}

#[tokio::test]
async fn a_source_that_ends_is_restarted_and_the_numbering_goes_on() {
    if !have_ffmpeg() {
        return;
    }
    // La source s'arrete apres une seconde : le superviseur la relance.
    let source = FfmpegFrameSource::spawn(fast(lavfi("320x180", 10, &["-t", "1"])));
    let first = source.next_after(0, Duration::from_secs(15)).await.unwrap();
    let mut later = first.seq;
    for _ in 0..40 {
        if source.status().starts >= 2 {
            if let Some(f) = source.next_after(later, Duration::from_secs(5)).await {
                later = f.seq;
                if later > first.seq + 4 {
                    break;
                }
            }
        } else {
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
    }
    assert!(
        source.status().starts >= 2,
        "relance : {:?}",
        source.status()
    );
    assert!(later > first.seq, "les numeros continuent apres la relance");
}

#[tokio::test]
async fn a_failing_input_is_retried_with_a_reason_and_a_missing_binary_is_explained() {
    // Entree introuvable : ffmpeg s'arrete, la cause est gardee, les tentatives continuent.
    let mut cfg = fast(vec!["-i".into(), "/nonexistent.mp4".into()]);
    cfg.ffmpeg = "ffmpeg".into();
    if have_ffmpeg() {
        let source = FfmpegFrameSource::spawn(cfg);
        tokio::time::sleep(Duration::from_millis(1500)).await;
        let st = source.status();
        assert!(st.starts >= 2, "{st:?}");
        assert!(!st.last_error.is_empty(), "{st:?}");
        assert!(source.latest().is_none());
    }
    // Pas de binaire du tout.
    let mut missing = fast(vec!["-i".into(), "x".into()]);
    missing.ffmpeg = "/nonexistent/ffmpeg".into();
    let source = FfmpegFrameSource::spawn(missing);
    tokio::time::sleep(Duration::from_millis(400)).await;
    assert!(
        source.status().last_error.contains("does not start"),
        "{:?}",
        source.status()
    );
}

#[tokio::test]
async fn dropping_the_source_stops_ffmpeg() {
    if !have_ffmpeg() {
        return;
    }
    let source = FfmpegFrameSource::spawn(fast(lavfi("322x182", 10, &[])));
    source.next_after(0, Duration::from_secs(15)).await.unwrap();
    let pids_before = ffmpeg_count();
    drop(source);
    for _ in 0..40 {
        if ffmpeg_count() < pids_before {
            return;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("ffmpeg tourne encore");
}

/// Nombre de processus `ffmpeg` qui portent une source `lavfi testsrc` (un format que seul ce test emploie : les autres tests tournent en meme temps).
fn ffmpeg_count() -> usize {
    let out = std::process::Command::new("pgrep")
        .args(["-fc", "ffmpeg.*322x182"])
        .output();
    out.ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(0)
}
