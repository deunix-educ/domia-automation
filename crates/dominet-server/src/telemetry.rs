// SPDX-License-Identifier: AGPL-3.0-or-later
//! Structured logging (`tracing`): readable text in dev, JSON in production.
//! The filter is set by `RUST_LOG` (default `info`). The same events feed the `/admin/logs` buffer (see `logbuf`).

use tracing_subscriber::prelude::*;
use tracing_subscriber::{EnvFilter, Layer};

use crate::config::LogFormat;
use crate::logbuf::LogLayer;

/// Installs the global logger. No effect if a logger is already installed (tests).
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
