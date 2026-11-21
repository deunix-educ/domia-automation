// SPDX-License-Identifier: AGPL-3.0-or-later
//! Integrations qui lisent une source externe et la publient sur MQTT (R3 lot 6) : Open-Meteo, SolarMax. L'energie (R4) et la
//! generation de tableaux Grafana viendront ici aussi.

pub mod run;
pub mod solarmax;
pub mod weather;
