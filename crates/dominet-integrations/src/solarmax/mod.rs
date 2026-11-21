// SPDX-License-Identifier: AGPL-3.0-or-later
//! Onduleur SolarMax : protocole pur, client TCP, simulateur.

pub mod client;
pub mod protocol;
pub mod sim;

pub use client::{QUERY_MASK, SolarMaxClient, SolarMaxError};
pub use protocol::{Production, ProtocolError, Value};
