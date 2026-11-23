// SPDX-License-Identifier: AGPL-3.0-or-later
//! SolarMax inverter: pure protocol, TCP client, simulator.

pub mod client;
pub mod protocol;
pub mod sim;

pub use client::{QUERY_MASK, SolarMaxClient, SolarMaxError};
pub use protocol::{Production, ProtocolError, Value};
