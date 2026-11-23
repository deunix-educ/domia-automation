// SPDX-License-Identifier: AGPL-3.0-or-later
//! Integrations that read an external source and publish it on MQTT (R3 lot 6): Open-Meteo, SolarMax. Energy (R4) and the
//! generation of Grafana dashboards will come here too.

pub mod run;
pub mod solarmax;
pub mod weather;
