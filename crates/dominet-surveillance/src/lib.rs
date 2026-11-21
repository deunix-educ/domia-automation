// SPDX-License-Identifier: AGPL-3.0-or-later
//! Cameras : MediaMTX, ONVIF, night mode, enregistrement, mouvement, YOLO.

pub mod frames;
pub mod mediamtx;
pub mod motion;
pub mod night;
pub mod onvif;
pub mod recording;
#[cfg(feature = "testing")]
pub mod testing;
pub mod yolo;

pub use frames::{FfmpegFrameSource, Frame, FrameConfig, FrameSource};
pub use mediamtx::{MediaMtx, MediaMtxError, PathConf, SyncReport};
