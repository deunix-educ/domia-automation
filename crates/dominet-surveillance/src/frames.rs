// SPDX-License-Identifier: AGPL-3.0-or-later
//! `FrameSource` (R6 lot 6, ADR 0003 et 0048) : des images decodees d'un flux video, a faible cadence, pour les snapshots, le mouvement
//! et la detection d'objets.
//!
//! Implementation : **un `ffmpeg` de longue duree par camera**, qui lit le **re-stream de MediaMTX** (jamais la camera directe), decode,
//! ramene la cadence a `fps` et la largeur a `max_width`, et ecrit des **JPEG** sur sa sortie standard. Un JPEG porte ses dimensions
//! (on n'a pas a les deviner d'apres ONVIF, qui se trompe parfois) et sert tel quel de snapshot ; le mouvement et YOLO le decodent.
//!
//! Le processus est **supervise** : redemarrage avec delai croissant, redemarrage d'un flux qui ne donne plus rien, et repli sur le
//! decodage logiciel si le decodage materiel (VAAPI) echoue.

use std::path::Path;
use std::process::Stdio;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use chrono::{DateTime, Utc};
use tokio::io::AsyncReadExt;
use tokio::process::Command;
use tokio::sync::{broadcast, watch};
use tokio::task::JoinHandle;
use tokio::time::Instant;

/// Une image : un JPEG complet, ses dimensions, son numero d'ordre (croissant dans une source) et l'heure de reception.
#[derive(Debug, Clone)]
pub struct Frame {
    pub jpeg: Arc<Vec<u8>>,
    pub width: u32,
    pub height: u32,
    pub seq: u64,
    pub at: DateTime<Utc>,
}

/// Ce que consomment le mouvement, YOLO et les snapshots.
pub trait FrameSource {
    /// Derniere image recue (`None` tant que le flux n'a rien donne).
    fn latest(&self) -> Option<Frame>;
    /// Toutes les images a venir.
    fn subscribe(&self) -> broadcast::Receiver<Frame>;
}

/// Decodage materiel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Hwaccel {
    /// VAAPI si `/dev/dri/renderD128` existe et n'a pas deja echoue, sinon logiciel.
    Auto,
    Off,
}

#[derive(Debug, Clone)]
pub struct FrameConfig {
    /// Arguments d'entree de `ffmpeg`, `-i` compris (ex. `["-rtsp_transport", "tcp", "-i", url]`) ; les tests y mettent une source `lavfi`.
    pub input: Vec<String>,
    /// Images par seconde en sortie.
    pub fps: f64,
    /// Largeur maximale (jamais agrandie) ; la hauteur suit, en nombre pair.
    pub max_width: u32,
    /// Qualite JPEG de `ffmpeg` (`-q:v`, 2 = meilleur, 31 = pire).
    pub quality: u8,
    pub hwaccel: Hwaccel,
    pub ffmpeg: String,
    /// Delai avant la premiere nouvelle tentative, puis doublant jusqu'a `backoff_max`.
    pub backoff_min: Duration,
    pub backoff_max: Duration,
    /// Un flux qui ne donne plus d'image pendant ce temps est redemarre.
    pub stall_after: Duration,
}

impl FrameConfig {
    /// Parametres de D7 : 2 images par seconde, 1280 de large.
    #[must_use]
    pub fn new(input: Vec<String>) -> Self {
        Self {
            input,
            fps: 2.0,
            max_width: 1280,
            quality: 5,
            hwaccel: Hwaccel::Auto,
            ffmpeg: "ffmpeg".into(),
            backoff_min: Duration::from_secs(1),
            backoff_max: Duration::from_secs(30),
            stall_after: Duration::from_secs(20),
        }
    }

    /// Entree pour un flux RTSP lu en TCP.
    #[must_use]
    pub fn rtsp_input(url: &str) -> Vec<String> {
        ["-rtsp_transport", "tcp", "-i", url]
            .map(String::from)
            .to_vec()
    }
}

const VAAPI_DEVICE: &str = "/dev/dri/renderD128";

/// Les arguments complets de `ffmpeg`. `vaapi` : decodage materiel (`hwdownload` puis echelle logicielle : `scale_vaapi` ne marche pas
/// sur cet hote, voir ADR 0003).
#[must_use]
pub fn ffmpeg_args(cfg: &FrameConfig, vaapi: bool) -> Vec<String> {
    let mut a: Vec<String> = ["-hide_banner", "-loglevel", "error", "-nostdin"]
        .map(String::from)
        .to_vec();
    if vaapi {
        a.extend(
            [
                "-hwaccel",
                "vaapi",
                "-hwaccel_device",
                VAAPI_DEVICE,
                "-hwaccel_output_format",
                "vaapi",
            ]
            .map(String::from),
        );
    } else {
        // Un seul fil de decodage : le budget CPU est le goulot, pas le debit (ADR 0003).
        a.extend(["-threads", "1"].map(String::from));
    }
    a.extend(cfg.input.iter().cloned());
    // `fps` avant `hwdownload` : c'est lui qui rend la copie GPU -> CPU rentable.
    let hw = if vaapi { "hwdownload,format=nv12," } else { "" };
    let filter = format!(
        "fps={},{hw}scale='min({},iw)':-2,format=yuvj420p",
        cfg.fps, cfg.max_width
    );
    a.extend(
        [
            "-an",
            "-vf",
            &filter,
            "-q:v",
            &cfg.quality.to_string(),
            "-f",
            "image2pipe",
            "-vcodec",
            "mjpeg",
            "pipe:1",
        ]
        .map(String::from),
    );
    a
}

// ------------------------------------------------------------------ decoupage des JPEG

/// Decoupe un flux d'octets en JPEG complets (`FF D8` ... `FF D9`). Dans un JPEG, un octet `FF` du contenu est toujours suivi de `00` ou
/// d'un marqueur : un `FF D9` ne se trouve donc qu'a la fin d'une image.
#[derive(Debug, Default)]
pub struct JpegSplitter {
    buf: Vec<u8>,
}

impl JpegSplitter {
    /// Ajoute des octets ; rend les images terminees (dans l'ordre).
    pub fn push(&mut self, bytes: &[u8]) -> Vec<Vec<u8>> {
        self.buf.extend_from_slice(bytes);
        let mut out = vec![];
        loop {
            // Debut de l'image : on jette ce qui precede (flux coupe en plein milieu d'une image).
            let Some(start) = find(&self.buf, [0xFF, 0xD8], 0) else {
                // Garder le dernier octet : il peut etre le `FF` d'un `FF D8` a venir.
                let keep = usize::from(self.buf.last() == Some(&0xFF));
                self.buf.drain(..self.buf.len() - keep);
                break;
            };
            let Some(end) = find(&self.buf, [0xFF, 0xD9], start + 2) else {
                self.buf.drain(..start);
                break;
            };
            out.push(self.buf[start..end + 2].to_vec());
            self.buf.drain(..end + 2);
        }
        // Garde-fou : un flux qui ne finit jamais une image ne doit pas remplir la memoire.
        if self.buf.len() > 32 * 1024 * 1024 {
            self.buf.clear();
        }
        out
    }
}

fn find(hay: &[u8], needle: [u8; 2], from: usize) -> Option<usize> {
    hay.get(from..)?
        .windows(2)
        .position(|w| w == needle.as_slice())
        .map(|p| p + from)
}

/// Largeur et hauteur d'un JPEG (marqueur `SOFn`), sans le decoder.
#[must_use]
pub fn jpeg_size(data: &[u8]) -> Option<(u32, u32)> {
    let mut i = 2;
    while i + 9 < data.len() {
        if data[i] != 0xFF {
            return None;
        }
        let marker = data[i + 1];
        // Marqueurs sans longueur.
        if matches!(marker, 0x01 | 0xD0..=0xD8) {
            i += 2;
            continue;
        }
        let len = usize::from(u16::from_be_bytes([data[i + 2], data[i + 3]]));
        // SOF0..SOF15 sauf DHT (C4), JPG (C8) et DAC (CC).
        if (0xC0..=0xCF).contains(&marker) && !matches!(marker, 0xC4 | 0xC8 | 0xCC) {
            let height = u32::from(u16::from_be_bytes([data[i + 5], data[i + 6]]));
            let width = u32::from(u16::from_be_bytes([data[i + 7], data[i + 8]]));
            return Some((width, height));
        }
        i += 2 + len;
    }
    None
}

// ------------------------------------------------------------------ source ffmpeg

/// Etat d'une source, pour l'administration et les tests.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SourceStatus {
    /// Un `ffmpeg` tourne.
    pub running: bool,
    /// Nombre de demarrages de `ffmpeg` (le premier compte).
    pub starts: u32,
    /// Le decodage materiel est (ou a ete) utilise.
    pub vaapi: bool,
    /// Derniere cause d'arret (sans secret).
    pub last_error: String,
}

/// Un `ffmpeg` supervise qui produit des [`Frame`]. Arrete a la destruction.
#[derive(Debug)]
pub struct FfmpegFrameSource {
    latest: watch::Receiver<Option<Frame>>,
    tx: broadcast::Sender<Frame>,
    status: watch::Receiver<SourceStatus>,
    task: JoinHandle<()>,
}

impl Drop for FfmpegFrameSource {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl FrameSource for FfmpegFrameSource {
    fn latest(&self) -> Option<Frame> {
        self.latest.borrow().clone()
    }

    fn subscribe(&self) -> broadcast::Receiver<Frame> {
        self.tx.subscribe()
    }
}

impl FfmpegFrameSource {
    /// Demarre la supervision (le premier `ffmpeg` part tout de suite).
    #[must_use]
    pub fn spawn(cfg: FrameConfig) -> Self {
        let (latest_tx, latest) = watch::channel(None);
        let (tx, _) = broadcast::channel(16);
        let (status_tx, status) = watch::channel(SourceStatus::default());
        let task = tokio::spawn(supervise(cfg, latest_tx, tx.clone(), status_tx));
        Self {
            latest,
            tx,
            status,
            task,
        }
    }

    #[must_use]
    pub fn status(&self) -> SourceStatus {
        self.status.borrow().clone()
    }

    /// La premiere image **posterieure** a `after` (numero d'ordre), ou `None` apres `timeout` : « une image fraiche ».
    pub async fn next_after(&self, after: u64, timeout: Duration) -> Option<Frame> {
        let mut rx = self.latest.clone();
        tokio::time::timeout(timeout, async move {
            loop {
                if let Some(f) = rx.borrow_and_update().clone()
                    && f.seq > after
                {
                    return Some(f);
                }
                if rx.changed().await.is_err() {
                    return None;
                }
            }
        })
        .await
        .ok()
        .flatten()
    }
}

async fn supervise(
    cfg: FrameConfig,
    latest: watch::Sender<Option<Frame>>,
    tx: broadcast::Sender<Frame>,
    status: watch::Sender<SourceStatus>,
) {
    let seq = AtomicU64::new(0);
    let mut delay = cfg.backoff_min;
    // Le materiel a-t-il deja echoue ? Alors logiciel pour toujours (une camera ne change pas de codec en route).
    let mut vaapi_failed = cfg.hwaccel == Hwaccel::Off || !Path::new(VAAPI_DEVICE).exists();
    let mut vaapi_strikes = 0u32;
    tracing::info!(
        decodage = if vaapi_failed { "software" } else { "VAAPI" },
        peripherique = Path::new(VAAPI_DEVICE).exists(),
        "video decoding of a source"
    );
    loop {
        let vaapi = !vaapi_failed;
        status.send_modify(|s| {
            s.running = true;
            s.starts += 1;
            s.vaapi = vaapi;
        });
        let started = Instant::now();
        let outcome = run_once(&cfg, vaapi, &seq, &latest, &tx).await;
        let produced = outcome.frames > 0;
        status.send_modify(|s| {
            s.running = false;
            s.last_error.clone_from(&outcome.error);
        });
        if produced {
            vaapi_strikes = 0;
        }
        let condemned = vaapi
            && !produced
            && match hardware_verdict(&outcome.stderr) {
                HardwareVerdict::Unusable => true,
                HardwareVerdict::DecodeFailed => {
                    vaapi_strikes += 1;
                    vaapi_strikes >= VAAPI_STRIKES
                }
                HardwareVerdict::NotHardware => false,
            };
        if condemned {
            // Le materiel lui-meme est en cause : logiciel pour de bon, sans attendre. Une camera hors ligne, un flux pas encore publie ou
            // un reseau en panne ne prouvent rien contre VAAPI (mesure sur l'EliteDesk : une camera absente au demarrage coupait le VAAPI).
            vaapi_failed = true;
            tracing::info!(
                cause = %outcome.stderr.replace('\n', " | "),
                "VAAPI decoding unusable, falling back to software decoding"
            );
            continue;
        }
        // Une source qui a tenu un moment repart vite ; une source qui echoue en boucle espace ses tentatives.
        delay = if produced && started.elapsed() > Duration::from_secs(10) {
            cfg.backoff_min
        } else {
            (delay * 2).min(cfg.backoff_max)
        };
        tokio::time::sleep(delay).await;
    }
}

struct Outcome {
    frames: u64,
    error: String,
    /// Tout ce que `ffmpeg` a ecrit sur sa sortie d'erreur (pour distinguer une panne materielle d'une panne de flux).
    stderr: String,
}

/// Ce que dit la sortie d'erreur de `ffmpeg` apres un decodage materiel sans image.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HardwareVerdict {
    /// Le materiel est inutilisable (pas de peripherique, pilote absent, format non gere) : logiciel pour de bon, tout de suite.
    Unusable,
    /// Le decodeur materiel a echoue sur ce flux (images de reference manquantes quand on rejoint un flux en cours de route, par
    /// exemple) : cela peut passer au flux suivant ; on ne condamne le materiel qu'apres plusieurs echecs d'affilee.
    DecodeFailed,
    /// Rien n'accuse le materiel (camera hors ligne, flux pas encore publie, reseau).
    NotHardware,
}

/// Echecs de decodage materiel d'affilee, sans aucune image, au-dela desquels on passe au logiciel.
const VAAPI_STRIKES: u32 = 3;

/// Classe la sortie d'erreur de `ffmpeg`.
#[must_use]
pub fn hardware_verdict(stderr: &str) -> HardwareVerdict {
    const UNUSABLE: [&str; 6] = [
        "no va display",
        "failed to initialise",
        "device creation failed",
        "/dev/dri",
        "function not implemented",
        "impossible to convert between the formats",
    ];
    const DECODE: [&str; 4] = ["vaapi", "hwaccel", "hwdownload", "failed to sync surface"];
    let lower = stderr.to_ascii_lowercase();
    if UNUSABLE.iter().any(|m| lower.contains(m)) {
        HardwareVerdict::Unusable
    } else if DECODE.iter().any(|m| lower.contains(m)) {
        HardwareVerdict::DecodeFailed
    } else {
        HardwareVerdict::NotHardware
    }
}

async fn run_once(
    cfg: &FrameConfig,
    vaapi: bool,
    seq: &AtomicU64,
    latest: &watch::Sender<Option<Frame>>,
    tx: &broadcast::Sender<Frame>,
) -> Outcome {
    let mut child = match Command::new(&cfg.ffmpeg)
        .args(ffmpeg_args(cfg, vaapi))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
    {
        Ok(c) => c,
        Err(e) => {
            return Outcome {
                frames: 0,
                error: format!("ffmpeg does not start: {e}"),
                stderr: String::new(),
            };
        }
    };
    let (Some(mut stdout), Some(mut stderr)) = (child.stdout.take(), child.stderr.take()) else {
        return Outcome {
            frames: 0,
            error: "ffmpeg outputs unavailable".into(),
            stderr: String::new(),
        };
    };
    // Dernieres lignes d'erreur de ffmpeg, pour dire pourquoi il s'arrete.
    let stderr_task = tokio::spawn(async move {
        let mut text = String::new();
        let _ = stderr.read_to_string(&mut text).await;
        text
    });
    let mut splitter = JpegSplitter::default();
    let mut chunk = vec![0u8; 64 * 1024];
    let mut frames = 0u64;
    let mut error = String::new();
    loop {
        match tokio::time::timeout(cfg.stall_after, stdout.read(&mut chunk)).await {
            Err(_) => {
                error = format!("no frame for {} s", cfg.stall_after.as_secs());
                break;
            }
            Ok(Err(e)) => {
                error = format!("reading ffmpeg: {e}");
                break;
            }
            Ok(Ok(0)) => break,
            Ok(Ok(n)) => {
                for jpeg in splitter.push(&chunk[..n]) {
                    let Some((width, height)) = jpeg_size(&jpeg) else {
                        continue;
                    };
                    frames += 1;
                    let frame = Frame {
                        jpeg: Arc::new(jpeg),
                        width,
                        height,
                        seq: seq.fetch_add(1, Ordering::Relaxed) + 1,
                        at: Utc::now(),
                    };
                    let _ = latest.send(Some(frame.clone()));
                    let _ = tx.send(frame);
                }
            }
        }
    }
    let _ = child.kill().await;
    let _ = child.wait().await;
    let text = stderr_task.await.unwrap_or_default();
    if error.is_empty() {
        let last = text
            .lines()
            .rev()
            .find(|l| !l.trim().is_empty())
            .unwrap_or("ffmpeg stopped");
        error = crate::onvif::mask_secrets(last, &[])
            .chars()
            .take(200)
            .collect();
    }
    Outcome {
        frames,
        error,
        stderr: text,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn jpeg(width: u16, height: u16, payload: &[u8]) -> Vec<u8> {
        // SOI, SOF0 (precision 8, hauteur, largeur, 1 composante), donnees (avec un FF 00 echappe), EOI.
        let mut v = vec![0xFF, 0xD8, 0xFF, 0xC0, 0x00, 0x0B, 0x08];
        v.extend(height.to_be_bytes());
        v.extend(width.to_be_bytes());
        v.extend([0x01, 0x01, 0x11, 0x00]);
        v.extend([0xFF, 0x00]);
        v.extend(payload);
        v.extend([0xFF, 0xD9]);
        v
    }

    #[test]
    fn a_missing_stream_never_condemns_vaapi_and_a_joined_gop_only_after_strikes() {
        use HardwareVerdict::{DecodeFailed, NotHardware, Unusable};
        assert_eq!(
            hardware_verdict(
                "[AVHWDeviceContext] No VA display found for device /dev/dri/renderD128."
            ),
            Unusable
        );
        assert_eq!(
            hardware_verdict("Error reinitializing filters!\nFunction not implemented"),
            Unusable
        );
        assert_eq!(
            hardware_verdict("Failed to initialise VAAPI connection: -1"),
            Unusable
        );
        // Flux rejoint en cours de route : le decodeur materiel n'a pas ses images de reference.
        assert_eq!(
            hardware_verdict(
                "co located POCs unavailable\nFailed to sync surface 0x4: 23 (internal decoding error).\nFailed to download frame: -5."
            ),
            DecodeFailed
        );
        assert_eq!(
            hardware_verdict("rtsp://127.0.0.1:8554/abc: Server returned 404 Not Found"),
            NotHardware
        );
        assert_eq!(
            hardware_verdict("Connection refused\nError opening input"),
            NotHardware
        );
        assert_eq!(hardware_verdict(""), NotHardware);
    }

    #[test]
    fn the_splitter_yields_whole_images_whatever_the_chunking() {
        let a = jpeg(640, 360, b"aaa");
        let b = jpeg(320, 180, b"bbbbbb");
        let all: Vec<u8> = [b"garbage".to_vec(), a.clone(), b"xx".to_vec(), b.clone()].concat();
        for chunk_size in [1, 2, 3, 7, 64, all.len()] {
            let mut s = JpegSplitter::default();
            let mut got = vec![];
            for piece in all.chunks(chunk_size) {
                got.extend(s.push(piece));
            }
            assert_eq!(got, [a.clone(), b.clone()], "paquets de {chunk_size}");
        }
    }

    #[test]
    fn a_stream_cut_in_the_middle_of_an_image_resumes_at_the_next_one() {
        let a = jpeg(100, 50, b"zz");
        let mut s = JpegSplitter::default();
        // Fin d'une image dont on a rate le debut, puis une image entiere.
        let mut data = a[10..].to_vec();
        data.extend(a.clone());
        assert_eq!(s.push(&data), [a]);
    }

    #[test]
    fn the_size_comes_from_the_sof_marker() {
        assert_eq!(jpeg_size(&jpeg(1280, 720, b"x")), Some((1280, 720)));
        assert_eq!(jpeg_size(&jpeg(640, 360, b"x")), Some((640, 360)));
        assert_eq!(jpeg_size(b"pas un jpeg"), None);
        assert_eq!(jpeg_size(&[0xFF, 0xD8, 0xFF, 0xD9]), None);
    }

    #[test]
    fn the_ffmpeg_command_follows_adr_0003() {
        let cfg = FrameConfig::new(FrameConfig::rtsp_input("rtsp://mediamtx:8554/abc"));
        let soft = ffmpeg_args(&cfg, false).join(" ");
        assert!(
            soft.contains("-threads 1 -rtsp_transport tcp -i rtsp://mediamtx:8554/abc"),
            "{soft}"
        );
        assert!(
            soft.contains("fps=2,scale='min(1280,iw)':-2,format=yuvj420p"),
            "{soft}"
        );
        assert!(soft.ends_with("-f image2pipe -vcodec mjpeg pipe:1") && !soft.contains("vaapi"));
        let hw = ffmpeg_args(&cfg, true).join(" ");
        assert!(
            hw.contains(
                "-hwaccel vaapi -hwaccel_device /dev/dri/renderD128 -hwaccel_output_format vaapi"
            ),
            "{hw}"
        );
        assert!(
            hw.contains("fps=2,hwdownload,format=nv12,scale="),
            "fps avant hwdownload : {hw}"
        );
        assert!(!hw.contains("-threads"), "{hw}");
    }
}
