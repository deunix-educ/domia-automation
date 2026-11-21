// SPDX-License-Identifier: AGPL-3.0-or-later
//! Journalisation structuree (`tracing`) : texte lisible en dev, JSON en production.
//! Le filtre se regle par `RUST_LOG` (defaut `info`). Les memes evenements alimentent le tampon de `/admin/logs` (voir `logbuf`).

use tracing_subscriber::prelude::*;
use tracing_subscriber::{EnvFilter, Layer};

use crate::config::LogFormat;
use crate::logbuf::LogLayer;

/// Installe le journal global. Sans effet si un journal est deja installe (tests).
pub fn init(format: LogFormat) {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    let console = match format {
        LogFormat::Pretty => tracing_subscriber::fmt::layer().with_target(true).boxed(),
        LogFormat::Json => tracing_subscriber::fmt::layer()
            .json()
            .flatten_event(true)
            .with_target(true)
            .boxed(),
    };
    let _ = tracing_subscriber::registry()
        .with(filter)
        .with(console)
        .with(LogLayer)
        .try_init();
}
