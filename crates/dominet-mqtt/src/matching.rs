// SPDX-License-Identifier: AGPL-3.0-or-later
//! Correspondance entre un filtre d'abonnement MQTT (`+` un niveau, `#` suffixe) et un topic. Code du spike 8 (ADR 0005), qui
//! diverge volontairement de `rumqttc::matches` sur les topics `$SYS`.

/// Vrai si `topic` correspond a `filter`. Un topic `$…` ne correspond jamais a un filtre qui commence par un joker.
#[must_use]
pub fn topic_matches(filter: &str, topic: &str) -> bool {
    if topic.starts_with('$') && (filter.starts_with('+') || filter.starts_with('#')) {
        return false;
    }
    let mut f = filter.split('/');
    let mut t = topic.split('/');
    loop {
        match (f.next(), t.next()) {
            (Some("#"), _) | (None, None) => return true,
            (Some("+"), Some(_)) => {}
            (Some(a), Some(b)) if a == b => {}
            _ => return false,
        }
    }
}

/// Vrai si `topic` correspond a au moins un des filtres.
pub fn matches_any<'a>(filters: impl IntoIterator<Item = &'a str>, topic: &str) -> bool {
    filters.into_iter().any(|f| topic_matches(f, topic))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_spike_cases_hold() {
        let cases: &[(&str, &str, bool)] = &[
            ("zigbee2mqtt/#", "zigbee2mqtt/Bureau", true),
            ("zigbee2mqtt/#", "zigbee2mqtt/bridge/devices", true),
            ("zigbee2mqtt/#", "zigbee2mqtt", true),
            ("zigbee2mqtt/+", "zigbee2mqtt/Bureau/availability", false),
            (
                "zigbee2mqtt/+/availability",
                "zigbee2mqtt/Bureau/availability",
                true,
            ),
            ("zigbee2mqtt/+/availability", "zigbee2mqtt/Bureau", false),
            ("linky/#", "linky/papp", true),
            ("solarmax/+/+", "solarmax/0xabc/onduleur", true),
            ("solarmax/+/+", "solarmax/0xabc", false),
            ("#", "a/b/c", true),
            ("#", "$SYS/broker/uptime", false),
            ("+/b", "$SYS/b", false),
            ("$SYS/#", "$SYS/broker/uptime", true),
            ("a//b", "a//b", true),
            ("a/+/b", "a//b", true),
            ("a/b", "a/b/c", false),
            ("a/b/c", "a/b", false),
            ("meteo/+", "meteo/paris", true),
            ("Bureau", "bureau", false),
        ];
        for (filter, topic, want) in cases {
            assert_eq!(topic_matches(filter, topic), *want, "{filter} / {topic}");
        }
    }

    #[test]
    fn exact_filters_and_trailing_levels() {
        assert!(topic_matches("a/b", "a/b"));
        assert!(topic_matches("a/#", "a"), "# couvre aussi le niveau parent");
        assert!(!topic_matches("a/+", "a"));
        assert!(!topic_matches("", "a"));
    }

    #[test]
    fn any_of_several_filters() {
        let filters = ["zigbee2mqtt/#", "linky/#"];
        assert!(matches_any(filters, "linky/papp"));
        assert!(!matches_any(filters, "meteo/paris"));
        assert!(!matches_any([], "a"));
    }
}
