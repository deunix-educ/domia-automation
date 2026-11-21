// SPDX-License-Identifier: AGPL-3.0-or-later
//! The tools each regime offers: fewer, well described tools make a small model choose better (ROADMAP-agent §3.5).

use dominet_domain::mcp::Scope;

/// What the user is doing: everyday use, or configuring the installation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Regime {
    /// Reading and driving the house (`lecture`, `pilotage`).
    Daily,
    /// Configuration and administration, plus the reads that help to describe an item.
    Config,
}

/// Reads kept in the configuration regime: what the model needs to name what it configures.
const CONFIG_READS: &[&str] = &[
    "config_kinds",
    "config_schema",
    "config_list",
    "config_get",
    "devices_list",
    "scenes_list",
    "automations_list",
];

impl Regime {
    /// Whether the regime offers `tool`, whose scope is `scope`.
    #[must_use]
    pub fn offers(self, tool: &str, scope: Scope) -> bool {
        match self {
            Self::Daily => matches!(scope, Scope::Read | Scope::Control),
            Self::Config => {
                matches!(scope, Scope::Configuration | Scope::Administration)
                    || CONFIG_READS.contains(&tool)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_regime_offers_its_own_scopes() {
        assert!(Regime::Daily.offers("device_command", Scope::Control));
        assert!(!Regime::Daily.offers("config_upsert", Scope::Configuration));
        assert!(Regime::Config.offers("config_upsert", Scope::Configuration));
        assert!(Regime::Config.offers("devices_list", Scope::Read));
        assert!(!Regime::Config.offers("logs_query", Scope::Read));
        assert!(!Regime::Config.offers("device_command", Scope::Control));
    }
}
