// SPDX-License-Identifier: AGPL-3.0-or-later
//! Regles d'affichage des widgets du tableau de bord, pures et testees sans navigateur (R2 lot 4) :
//! choix automatique de la presentation d'un appareil, lignes d'etat (unites), adresse d'un panneau Grafana.

use serde_json::Value;

use crate::dashboard::Presentation;
use crate::device::DeviceType;

/// Presentation d'un appareil : celle demandee ; pour `Auto`, celle de son **type** quand il est precis, sinon (type
/// `generic`, ou appareil Zigbee2MQTT dont les `exposes` composites n'ont pas permis de deviner le type) celle deduite de
/// son etat.
#[must_use]
pub fn presentation_for(
    requested: Presentation,
    kind: Option<DeviceType>,
    state: &Value,
) -> Presentation {
    if requested != Presentation::Auto {
        return requested;
    }
    match kind {
        Some(DeviceType::Sensor) => Presentation::DeviceSensor,
        Some(DeviceType::BinarySensor) => Presentation::DeviceBinarySensor,
        Some(DeviceType::Switch) => Presentation::DeviceSwitch,
        Some(DeviceType::Light) => Presentation::DeviceLight,
        Some(DeviceType::Cover) => Presentation::DeviceCover,
        Some(DeviceType::Climate) => Presentation::DeviceClimate,
        Some(DeviceType::Remote | DeviceType::Generic) | None => {
            resolve_presentation(Presentation::Auto, state)
        }
    }
}

/// Presentation a utiliser pour un appareil : celle demandee, ou (pour `Auto`) celle deduite des proprietes de son etat.
/// Faute de table d'appareils (R3), le type de l'appareil se lit dans les cles de l'etat publie.
#[must_use]
pub fn resolve_presentation(requested: Presentation, state: &Value) -> Presentation {
    if requested != Presentation::Auto {
        return requested;
    }
    let Some(o) = state.as_object() else {
        return Presentation::DeviceGeneric;
    };
    let has = |k: &str| o.contains_key(k);
    let any = |ks: &[&str]| ks.iter().any(|k| has(k));
    let text = |k: &str| o.get(k).and_then(Value::as_str);
    if any(&["brightness", "color_temp", "color"]) {
        Presentation::DeviceLight
    } else if any(&["position", "tilt"]) {
        Presentation::DeviceCover
    } else if any(&[
        "occupied_heating_setpoint",
        "current_heating_setpoint",
        "local_temperature",
        "system_mode",
    ]) {
        Presentation::DeviceClimate
    } else if any(&["fan_mode", "fan_state", "percentage"]) {
        Presentation::DeviceFan
    } else if has("lock_state") || matches!(text("state"), Some("LOCK" | "UNLOCK")) {
        Presentation::DeviceLock
    } else if any(&["condition", "forecast", "wind_speed"]) {
        Presentation::DeviceWeather
    } else if matches!(text("state"), Some("ON" | "OFF")) {
        Presentation::DeviceSwitch
    } else if !o.is_empty()
        && o.iter()
            .filter(|(k, _)| !is_diagnostic(k))
            .all(|(_, v)| v.is_boolean())
    {
        Presentation::DeviceBinarySensor
    } else if o
        .iter()
        .filter(|(k, _)| !is_diagnostic(k))
        .all(|(_, v)| v.is_number())
        && o.keys().any(|k| !is_diagnostic(k))
    {
        Presentation::DeviceSensor
    } else {
        Presentation::DeviceGeneric
    }
}

/// Proprietes de diagnostic, masquees du choix de presentation et placees en fin de liste.
fn is_diagnostic(key: &str) -> bool {
    matches!(
        key,
        "linkquality" | "battery" | "voltage" | "update" | "update_available" | "last_seen"
    )
}

/// Une ligne d'etat prete a afficher.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StateRow {
    pub key: String,
    /// Nom lisible de la cle (libelle de l'appareil) ; vide = a deduire de la cle.
    pub name: String,
    pub value: String,
    pub unit: String,
    pub is_bool: bool,
    /// Valeur booleenne vraie (significatif si `is_bool`).
    pub on: bool,
}

/// Unite usuelle d'une propriete Zigbee2MQTT.
#[must_use]
pub fn unit_of(key: &str) -> &'static str {
    match key {
        "temperature"
        | "local_temperature"
        | "occupied_heating_setpoint"
        | "current_heating_setpoint" => "°C",
        "humidity" | "battery" | "soil_moisture" | "position" | "percentage"
        | "brightness_percent" => "%",
        "pressure" => "hPa",
        "illuminance" | "illuminance_lux" => "lx",
        "power" => "W",
        "energy" => "kWh",
        "voltage" => "V",
        "current" => "A",
        "co2" => "ppm",
        "wind_speed" => "km/h",
        _ => "",
    }
}

fn format_number(n: &serde_json::Number) -> String {
    match n.as_f64() {
        Some(f) if f.fract() != 0.0 => {
            let s = format!("{f:.2}");
            s.trim_end_matches('0').trim_end_matches('.').to_string()
        }
        _ => n.to_string(),
    }
}

/// Lignes d'etat d'un appareil, triees par nom, diagnostic en dernier. Les objets et tableaux sont resumes en JSON compact
/// tronque (jamais de HTML : le texte est echappe par l'affichage).
#[must_use]
pub fn state_rows(state: &Value) -> Vec<StateRow> {
    state_rows_with(state, &[], &[])
}

/// Comme [`state_rows`], avec les libelles de la table des appareils (nom, unite) et les cles masquees de l'appareil.
/// Un libelle sans unite laisse l'unite usuelle de la cle.
#[must_use]
pub fn state_rows_with(
    state: &Value,
    labels: &[(String, String, String)],
    hidden: &[String],
) -> Vec<StateRow> {
    let Some(o) = state.as_object() else {
        return vec![];
    };
    let mut rows: Vec<StateRow> = o
        .iter()
        .filter(|(k, _)| !hidden.contains(k))
        .map(|(k, v)| {
            let label = labels.iter().find(|(key, _, _)| key == k);
            let (value, is_bool, on) = match v {
                Value::String(s) => (s.chars().take(80).collect(), false, false),
                Value::Number(n) => (format_number(n), false, false),
                Value::Bool(b) => (b.to_string(), true, *b),
                Value::Null => ("—".to_string(), false, false),
                other => {
                    let mut t = other.to_string();
                    if t.chars().count() > 80 {
                        t = t.chars().take(79).collect::<String>() + "…";
                    }
                    (t, false, false)
                }
            };
            let unit = if v.is_number() {
                label
                    .filter(|(_, _, u)| !u.is_empty())
                    .map_or_else(|| unit_of(k).to_string(), |(_, _, u)| u.clone())
            } else {
                String::new()
            };
            StateRow {
                key: k.clone(),
                name: label.map(|(_, n, _)| n.clone()).unwrap_or_default(),
                value,
                unit,
                is_bool,
                on,
            }
        })
        .collect();
    rows.sort_by(|a, b| (is_diagnostic(&a.key), &a.key).cmp(&(is_diagnostic(&b.key), &b.key)));
    rows
}

/// Origine (`schema://hote[:port]`) d'une adresse http(s), en minuscules ; `None` si ce n'en est pas une.
#[must_use]
pub fn http_origin(url: &str) -> Option<String> {
    let (scheme, rest) = url.split_once("://")?;
    if !matches!(scheme.to_ascii_lowercase().as_str(), "http" | "https") {
        return None;
    }
    let authority = rest.split(['/', '?', '#']).next()?;
    // Pas d'identifiants dans l'adresse (user:pass@hote) : refus plutot que contournement.
    if authority.is_empty() || authority.contains('@') || authority.contains(char::is_whitespace) {
        return None;
    }
    Some(format!(
        "{}://{}",
        scheme.to_ascii_lowercase(),
        authority.to_ascii_lowercase()
    ))
}

/// Adresse a charger dans l'iframe d'un widget Grafana.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GrafanaSrc {
    Url(String),
    /// Rien d'exploitable dans la configuration (ou `GRAFANA_PUBLIC_URL` vide pour un panneau par uid / appareil).
    NotConfigured,
    /// `panel_url` d'une autre origine que `GRAFANA_PUBLIC_URL` : la CSP la bloquerait, on ne la charge pas.
    Forbidden,
}

const KIOSK: &str = "kiosk=tv&theme=dark&refresh=30s";

fn with_query(url: &str, extra: &str) -> String {
    let sep = if url.contains('?') { '&' } else { '?' };
    format!("{url}{sep}{extra}")
}

/// Calcule l'adresse du panneau (memes cas que l'implementation de reference) : `panel_url`, sinon appareil + `key`, sinon
/// `dashboard_uid` (+ `panel_id`). `base` est `GRAFANA_PUBLIC_URL`. Seule l'origine de `base` est autorisee.
#[must_use]
pub fn grafana_src(config: &Value, device_id: Option<i64>, base: Option<&str>) -> GrafanaSrc {
    let base = base
        .map(|b| b.trim().trim_end_matches('/'))
        .filter(|b| !b.is_empty());
    let text = |k: &str| {
        config
            .get(k)
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
    };
    if let Some(panel) = text("panel_url") {
        let allowed = base.and_then(http_origin);
        return match (http_origin(panel), allowed) {
            (Some(o), Some(a)) if o == a => GrafanaSrc::Url(with_query(panel, KIOSK)),
            (Some(_), _) => GrafanaSrc::Forbidden,
            _ => GrafanaSrc::NotConfigured,
        };
    }
    let Some(base) = base else {
        return GrafanaSrc::NotConfigured;
    };
    if let (Some(id), Some(key)) = (device_id, text("key")) {
        let key: String = key
            .chars()
            .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-'))
            .collect();
        return GrafanaSrc::Url(format!(
            "{base}/d/dominet-device?var-device_id={id}&var-key={key}&{KIOSK}"
        ));
    }
    if let Some(uid) = text("dashboard_uid") {
        let uid: String = uid
            .chars()
            .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-'))
            .collect();
        let panel = config.get("panel_id").and_then(Value::as_u64);
        let head = match panel {
            Some(p) => format!("{base}/d/{uid}?viewPanel={p}&"),
            None => format!("{base}/d/{uid}?"),
        };
        return GrafanaSrc::Url(format!("{head}{KIOSK}"));
    }
    GrafanaSrc::NotConfigured
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn auto(state: &Value) -> Presentation {
        resolve_presentation(Presentation::Auto, state)
    }

    #[test]
    fn a_requested_presentation_always_wins() {
        assert_eq!(
            resolve_presentation(Presentation::DeviceFan, &json!({"brightness": 3})),
            Presentation::DeviceFan
        );
    }

    #[test]
    fn auto_infers_the_presentation_from_the_state_keys() {
        use Presentation as P;
        assert_eq!(
            auto(&json!({"state": "ON", "brightness": 120})),
            P::DeviceLight
        );
        assert_eq!(auto(&json!({"position": 40})), P::DeviceCover);
        assert_eq!(
            auto(&json!({"local_temperature": 19.5, "system_mode": "heat"})),
            P::DeviceClimate
        );
        assert_eq!(auto(&json!({"fan_mode": "auto"})), P::DeviceFan);
        assert_eq!(auto(&json!({"lock_state": "locked"})), P::DeviceLock);
        assert_eq!(
            auto(&json!({"condition": "sunny", "temperature": 21})),
            P::DeviceWeather
        );
        assert_eq!(auto(&json!({"state": "OFF", "power": 0})), P::DeviceSwitch);
        assert_eq!(
            auto(&json!({"contact": true, "battery": 90})),
            P::DeviceBinarySensor
        );
        assert_eq!(
            auto(&json!({"temperature": 21.5, "humidity": 40, "linkquality": 80})),
            P::DeviceSensor
        );
        assert_eq!(auto(&json!({"mode": "eco"})), P::DeviceGeneric);
        assert_eq!(auto(&json!({})), P::DeviceGeneric);
        assert_eq!(auto(&json!("pas un objet")), P::DeviceGeneric);
    }

    #[test]
    fn labels_name_and_unit_the_rows_and_hidden_keys_disappear() {
        let labels = vec![
            (
                "temperature".to_string(),
                "Température".to_string(),
                "K".to_string(),
            ),
            ("battery".to_string(), "Battery".to_string(), String::new()),
            ("power".to_string(), "Power".to_string(), "W".to_string()),
        ];
        let hidden = vec!["update".to_string()];
        let rows = state_rows_with(
            &json!({"temperature": 3, "battery": 80, "power": "pas un nombre", "update": {"state": "idle"}, "x": 1}),
            &labels,
            &hidden,
        );
        let by = |k: &str| rows.iter().find(|r| r.key == k).unwrap();
        assert!(rows.iter().all(|r| r.key != "update"), "cle masquee");
        // Le libelle donne le nom et, s'il en a une, l'unite ; sans unite, l'unite usuelle reste.
        assert_eq!(
            (
                by("temperature").name.as_str(),
                by("temperature").unit.as_str()
            ),
            ("Température", "K")
        );
        assert_eq!(
            (by("battery").name.as_str(), by("battery").unit.as_str()),
            ("Battery", "%")
        );
        // Pas d'unite pour une valeur qui n'est pas un nombre ; cle sans libelle : nom vide (deduit de la cle a l'affichage).
        assert_eq!(by("power").unit, "");
        assert_eq!((by("x").name.as_str(), by("x").unit.as_str()), ("", ""));
    }

    #[test]
    fn auto_uses_the_device_type_when_precise_and_the_state_otherwise() {
        use DeviceType as T;
        use Presentation as P;
        let state = json!({"state": "ON", "brightness": 10});
        let p = |kind| presentation_for(P::Auto, kind, &state);
        assert_eq!(p(Some(T::Sensor)), P::DeviceSensor);
        assert_eq!(p(Some(T::BinarySensor)), P::DeviceBinarySensor);
        assert_eq!(p(Some(T::Switch)), P::DeviceSwitch);
        assert_eq!(p(Some(T::Cover)), P::DeviceCover);
        assert_eq!(p(Some(T::Climate)), P::DeviceClimate);
        // Type generique (dont les lampes Zigbee2MQTT, voir ADR 0025) ou inconnu : l'etat decide.
        assert_eq!(p(Some(T::Generic)), P::DeviceLight);
        assert_eq!(p(Some(T::Remote)), P::DeviceLight);
        assert_eq!(p(None), P::DeviceLight);
        // La presentation demandee l'emporte toujours.
        assert_eq!(
            presentation_for(P::DeviceFan, Some(T::Sensor), &state),
            P::DeviceFan
        );
    }

    #[test]
    fn state_rows_have_units_and_diagnostics_last() {
        let rows = state_rows(
            &json!({"linkquality": 70, "temperature": 21.456, "contact": true, "name": "x", "nested": {"a": 1}}),
        );
        let keys: Vec<_> = rows.iter().map(|r| r.key.as_str()).collect();
        assert_eq!(
            keys,
            ["contact", "name", "nested", "temperature", "linkquality"]
        );
        let t = &rows[3];
        assert_eq!((t.value.as_str(), t.unit.as_str()), ("21.46", "°C"));
        assert!(rows[0].is_bool && rows[0].on);
        assert_eq!(rows[2].value, r#"{"a":1}"#);
        assert_eq!(state_rows(&json!(3)), []);
    }

    #[test]
    fn long_values_are_truncated() {
        let r = state_rows(&json!({"s": "a".repeat(500), "o": {"k": "b".repeat(500)}}));
        assert_eq!(
            r.iter()
                .find(|x| x.key == "s")
                .unwrap()
                .value
                .chars()
                .count(),
            80
        );
        assert_eq!(
            r.iter()
                .find(|x| x.key == "o")
                .unwrap()
                .value
                .chars()
                .count(),
            80
        );
    }

    #[test]
    fn origins_are_normalised_and_credentials_refused() {
        assert_eq!(
            http_origin("HTTPS://Grafana.Local:3000/d/x?y=1"),
            Some("https://grafana.local:3000".into())
        );
        assert_eq!(http_origin("http://a/"), Some("http://a".into()));
        assert_eq!(http_origin("javascript:alert(1)"), None);
        assert_eq!(http_origin("ftp://a/"), None);
        assert_eq!(http_origin("http://user:pw@a/"), None);
        assert_eq!(http_origin("http:///x"), None);
    }

    #[test]
    fn grafana_panel_urls_must_share_the_configured_origin() {
        let cfg = json!({"panel_url": "http://grafana:3000/d-solo/abc?panelId=2"});
        assert_eq!(
            grafana_src(&cfg, None, Some("http://grafana:3000/")),
            GrafanaSrc::Url(
                "http://grafana:3000/d-solo/abc?panelId=2&kiosk=tv&theme=dark&refresh=30s".into()
            )
        );
        assert_eq!(
            grafana_src(&cfg, None, Some("http://autre:3000")),
            GrafanaSrc::Forbidden
        );
        assert_eq!(grafana_src(&cfg, None, None), GrafanaSrc::Forbidden);
        assert_eq!(
            grafana_src(
                &json!({"panel_url": "javascript:1"}),
                None,
                Some("http://g")
            ),
            GrafanaSrc::NotConfigured
        );
    }

    #[test]
    fn grafana_panels_by_uid_or_by_device_use_the_public_url() {
        let b = Some("https://g.example/");
        assert_eq!(
            grafana_src(&json!({"dashboard_uid": "abc-1", "panel_id": 4}), None, b),
            GrafanaSrc::Url(
                "https://g.example/d/abc-1?viewPanel=4&kiosk=tv&theme=dark&refresh=30s".into()
            )
        );
        assert_eq!(
            grafana_src(&json!({"dashboard_uid": "abc-1"}), None, b),
            GrafanaSrc::Url("https://g.example/d/abc-1?kiosk=tv&theme=dark&refresh=30s".into())
        );
        assert_eq!(
            grafana_src(&json!({"key": "temperature"}), Some(7), b),
            GrafanaSrc::Url("https://g.example/d/dominet-device?var-device_id=7&var-key=temperature&kiosk=tv&theme=dark&refresh=30s".into())
        );
        assert_eq!(
            grafana_src(&json!({"dashboard_uid": "abc"}), None, None),
            GrafanaSrc::NotConfigured
        );
        assert_eq!(grafana_src(&json!({}), None, b), GrafanaSrc::NotConfigured);
        // Un uid ou une cle ne peut pas injecter de parametres.
        assert_eq!(
            grafana_src(&json!({"dashboard_uid": "a&x=1#"}), None, b),
            GrafanaSrc::Url("https://g.example/d/ax1?kiosk=tv&theme=dark&refresh=30s".into())
        );
    }
}
