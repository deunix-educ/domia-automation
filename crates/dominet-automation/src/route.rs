// SPDX-License-Identifier: AGPL-3.0-or-later
//! Aiguillage d'un message selon son topic, dans l'ordre de l'implementation de reference.

use dominet_mqtt::Payload;

/// Ce que le traitement doit faire d'un message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Route {
    /// `{base}/bridge/devices` : liste des appareils.
    BridgeDevices,
    /// `{base}/bridge/info` (charge utile objet) : etat du bridge, dont `permit_join`.
    BridgeInfo,
    /// `{base}/bridge/logging` (object payload): a log line of Zigbee2MQTT, errors are reported.
    BridgeLog,
    /// `{base}/{nom}/availability`.
    Availability(String),
    /// `{base}/{nom}` : etat d'un appareil.
    State(String),
    /// Autre sujet `{base}/bridge/…`, ou `…/set`, `…/get` : ignore.
    Ignored,
    /// Hors du prefixe Zigbee2MQTT : sources MQTT generiques.
    Other,
}

/// Aiguille `topic` pour le prefixe `base` (sans `/` final).
///
/// Ordre repris de la reference (il compte) : `bridge/devices`, `bridge/info` (objet), `…/availability` **avant** l'ecart des sujets
/// `bridge/…`, puis `…/set` et `…/get`, puis l'etat.
#[must_use]
pub fn route(base: &str, topic: &str, payload: &Payload) -> Route {
    let Some(rest) = topic.strip_prefix(base).and_then(|r| r.strip_prefix('/')) else {
        return Route::Other;
    };
    if rest == "bridge/devices" {
        return Route::BridgeDevices;
    }
    if rest == "bridge/info" && payload.as_object().is_some() {
        return Route::BridgeInfo;
    }
    if rest == "bridge/logging" && payload.as_object().is_some() {
        return Route::BridgeLog;
    }
    if let Some(name) = rest.strip_suffix("/availability") {
        return Route::Availability(name.to_string());
    }
    if rest.starts_with("bridge/") || rest.ends_with("/set") || rest.ends_with("/get") {
        return Route::Ignored;
    }
    Route::State(rest.to_string())
}

#[cfg(test)]
mod tests {
    use dominet_mqtt::decode_payload;

    use super::*;

    fn r(topic: &str, payload: &[u8]) -> Route {
        route("zigbee2mqtt", topic, &decode_payload(payload))
    }

    #[test]
    fn z2m_topics_are_routed_like_reference() {
        assert_eq!(r("zigbee2mqtt/bridge/devices", b"[]"), Route::BridgeDevices);
        assert_eq!(
            r("zigbee2mqtt/bridge/info", br#"{"permit_join":true}"#),
            Route::BridgeInfo
        );
        assert_eq!(
            r("zigbee2mqtt/Bureau/availability", b"online"),
            Route::Availability("Bureau".into())
        );
        assert_eq!(
            r("zigbee2mqtt/salon/lampe/availability", b"{}"),
            Route::Availability("salon/lampe".into())
        );
        assert_eq!(
            r("zigbee2mqtt/Bureau", b"{}"),
            Route::State("Bureau".into())
        );
        assert_eq!(
            r("zigbee2mqtt/salon/lampe", b"{}"),
            Route::State("salon/lampe".into())
        );
    }

    #[test]
    fn bridge_noise_and_commands_are_ignored() {
        assert_eq!(r("zigbee2mqtt/bridge/state", b"online"), Route::Ignored);
        assert_eq!(r("zigbee2mqtt/bridge/logs", b"{}"), Route::Ignored);
        assert_eq!(
            r(
                "zigbee2mqtt/bridge/logging",
                br#"{"level":"error","message":"x"}"#
            ),
            Route::BridgeLog
        );
        assert_eq!(r("zigbee2mqtt/bridge/logging", b"texte"), Route::Ignored);
        assert_eq!(r("zigbee2mqtt/Bureau/set", b"{}"), Route::Ignored);
        assert_eq!(r("zigbee2mqtt/Bureau/get", b"{}"), Route::Ignored);
        // `bridge/info` dont la charge n'est pas un objet : retombe sur l'ecart des sujets `bridge/…`.
        assert_eq!(r("zigbee2mqtt/bridge/info", b"texte"), Route::Ignored);
    }

    #[test]
    fn reference_order_quirks_are_kept() {
        // `availability` est teste avant l'ecart de `bridge/…` : un tel sujet vise un appareil nomme « bridge/x ».
        assert_eq!(
            r("zigbee2mqtt/bridge/x/availability", b"online"),
            Route::Availability("bridge/x".into())
        );
        // Un nom se terminant par `/set` est une commande, meme s'il ressemble a un appareil.
        assert_eq!(r("zigbee2mqtt/a/b/set", b"{}"), Route::Ignored);
    }

    #[test]
    fn other_topics_belong_to_the_generic_sources() {
        assert_eq!(r("linky/papp", b"{}"), Route::Other);
        assert_eq!(
            r("zigbee2mqttx/foo", b"{}"),
            Route::Other,
            "le prefixe doit etre suivi d'un `/`"
        );
        assert_eq!(r("zigbee2mqtt", b"{}"), Route::Other);
        assert_eq!(
            route("custom/base", "custom/base/Bureau", &decode_payload(b"{}")),
            Route::State("Bureau".into())
        );
    }
}
