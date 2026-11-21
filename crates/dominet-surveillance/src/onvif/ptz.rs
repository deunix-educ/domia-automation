// SPDX-License-Identifier: AGPL-3.0-or-later
//! PTZ ONVIF (R6 lot 5, ADR 0047) : `ContinuousMove` / `Stop` (les seuls mouvements de l'interface), positions, presets, `AbsoluteMove`.
//!
//! Une [`PtzControl`] tient l'adresse du service PTZ et le jeton du profil : elles sont lues une fois (`GetCapabilities`, `GetProfiles`)
//! puis reutilisees pendant toute une session de pilotage.

use std::fmt::Write;
use std::time::Duration;

use roxmltree::Node;

use super::{OnvifClient, OnvifError, find, parse, text, xml_escape};

/// Vitesse minimale acceptee (comme la reference : en dessous, plusieurs cameras ne bougent pas).
pub const MIN_SPEED: f64 = 0.05;

/// Une position PTZ, dans l'espace que la camera annonce (`space` : adresse de l'espace de coordonnees, si connue).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct PtzPosition {
    pub pan: f64,
    pub tilt: f64,
    pub zoom: f64,
    /// Espace des coordonnees pan/tilt (Thingino : degres, `SphericalPositionSpace`).
    pub space: Option<String>,
}

/// Une position PTZ enregistree dans la camera.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CameraPreset {
    pub token: String,
    pub name: String,
}

/// La camera est-elle au repos ?
#[derive(Debug, Clone, PartialEq, Default)]
pub struct PtzStatus {
    pub position: Option<PtzPosition>,
    /// `true` si `MoveStatus` annonce `IDLE` (ou si la camera ne le dit pas).
    pub idle: bool,
}

/// Ramene une vitesse dans `MIN_SPEED..=1.0` (une valeur illisible donne le milieu).
#[must_use]
pub fn clamp_speed(speed: f64) -> f64 {
    if speed.is_nan() {
        0.5
    } else {
        speed.clamp(MIN_SPEED, 1.0)
    }
}

/// Une composante de vitesse dans `-1.0..=1.0` (une valeur illisible vaut zero : on n'ordonne pas un mouvement incertain).
#[must_use]
pub fn clamp_axis(v: f64) -> f64 {
    if v.is_nan() { 0.0 } else { v.clamp(-1.0, 1.0) }
}

#[derive(Debug)]
pub struct PtzControl<'a> {
    client: &'a OnvifClient,
    url: String,
    profile: String,
}

impl OnvifClient {
    /// Prepare le pilotage PTZ : adresse du service PTZ et premier profil qui porte une configuration PTZ.
    ///
    /// # Errors
    /// `NoService("PTZ")` si la camera n'a pas de PTZ ; erreurs habituelles des appels ONVIF.
    pub async fn ptz(&self) -> Result<PtzControl<'_>, OnvifError> {
        let services = self.services().await?;
        let url = services.ptz.ok_or(OnvifError::NoService("PTZ"))?;
        let media = services.media.ok_or(OnvifError::NoService("Media"))?;
        let profile = self
            .profiles(&media)
            .await?
            .into_iter()
            .find(|p| p.has_ptz)
            .ok_or(OnvifError::NoService("PTZ profile"))?
            .token;
        Ok(PtzControl {
            client: self,
            url,
            profile,
        })
    }
}

fn number(n: Node, attr: &str) -> f64 {
    n.attribute(attr)
        .and_then(|v| v.parse().ok())
        .unwrap_or(0.0)
}

impl PtzControl<'_> {
    async fn call(&self, body: &str) -> Result<String, OnvifError> {
        self.client.authed(&self.url, body).await
    }

    fn token(&self) -> String {
        xml_escape(&self.profile)
    }

    /// Mouvement continu : `pan` et `tilt` dans `-1..=1`, `zoom` dans `-1..=1` (0 = pas de mouvement sur cet axe).
    ///
    /// # Errors
    /// Erreurs des appels ONVIF.
    pub async fn continuous_move(&self, pan: f64, tilt: f64, zoom: f64) -> Result<(), OnvifError> {
        let (pan, tilt, zoom) = (clamp_axis(pan), clamp_axis(tilt), clamp_axis(zoom));
        let mut velocity = String::new();
        // Comme la reference : seuls les axes en mouvement sont envoyes.
        if pan != 0.0 || tilt != 0.0 {
            let _ = write!(velocity, r#"<tt:PanTilt x="{pan}" y="{tilt}"/>"#);
        }
        if zoom != 0.0 {
            let _ = write!(velocity, r#"<tt:Zoom x="{zoom}"/>"#);
        }
        if velocity.is_empty() {
            return self.stop().await;
        }
        self.call(&format!(
            "<tptz:ContinuousMove><tptz:ProfileToken>{}</tptz:ProfileToken><tptz:Velocity>{velocity}</tptz:Velocity></tptz:ContinuousMove>",
            self.token()
        ))
        .await
        .map(|_| ())
    }

    /// Arrete tout mouvement (pan / tilt et zoom).
    ///
    /// # Errors
    /// Erreurs des appels ONVIF.
    pub async fn stop(&self) -> Result<(), OnvifError> {
        self.call(&format!(
            "<tptz:Stop><tptz:ProfileToken>{}</tptz:ProfileToken><tptz:PanTilt>true</tptz:PanTilt><tptz:Zoom>true</tptz:Zoom></tptz:Stop>",
            self.token()
        ))
        .await
        .map(|_| ())
    }

    /// Position et etat de mouvement (`None` pour la position si la camera ne la donne pas : la NVT renvoie toujours 0, 0).
    ///
    /// # Errors
    /// Erreurs des appels ONVIF.
    pub async fn status(&self) -> Result<PtzStatus, OnvifError> {
        let xml = self
            .call(&format!(
                "<tptz:GetStatus><tptz:ProfileToken>{}</tptz:ProfileToken></tptz:GetStatus>",
                self.token()
            ))
            .await?;
        let doc = parse(&xml)?;
        let position = find(doc.root(), "Position").map(|p| {
            let pt = find(p, "PanTilt");
            PtzPosition {
                pan: pt.map_or(0.0, |n| number(n, "x")),
                tilt: pt.map_or(0.0, |n| number(n, "y")),
                zoom: find(p, "Zoom").map_or(0.0, |n| number(n, "x")),
                space: pt.and_then(|n| n.attribute("space")).map(str::to_string),
            }
        });
        let idle = find(doc.root(), "MoveStatus").is_none_or(|ms| {
            ["PanTilt", "Zoom"].iter().all(|axis| {
                text(ms, axis)
                    .is_none_or(|v| matches!(v.to_ascii_uppercase().as_str(), "IDLE" | "NONE" | ""))
            })
        });
        Ok(PtzStatus { position, idle })
    }

    /// Attend la fin d'un mouvement (au plus `timeout`), comme la reference apres `GotoPreset` / `AbsoluteMove`.
    pub async fn wait_idle(&self, timeout: Duration) {
        let start = tokio::time::Instant::now();
        while start.elapsed() < timeout {
            match self.status().await {
                Ok(s) if !s.idle => tokio::time::sleep(Duration::from_millis(150)).await,
                _ => return,
            }
        }
    }

    /// Presets enregistres dans la camera.
    ///
    /// # Errors
    /// Erreurs des appels ONVIF.
    pub async fn presets(&self) -> Result<Vec<CameraPreset>, OnvifError> {
        let xml = self
            .call(&format!(
                "<tptz:GetPresets><tptz:ProfileToken>{}</tptz:ProfileToken></tptz:GetPresets>",
                self.token()
            ))
            .await?;
        let doc = parse(&xml)?;
        Ok(doc
            .root()
            .descendants()
            .filter(|d| d.is_element() && d.tag_name().name() == "Preset")
            .filter_map(|p| {
                Some(CameraPreset {
                    token: p.attribute("token")?.to_string(),
                    name: text(p, "Name").unwrap_or_default(),
                })
            })
            .collect())
    }

    /// Enregistre la position actuelle sous ce nom ; retourne le jeton donne par la camera (vide si elle ne le dit pas).
    ///
    /// # Errors
    /// Erreurs des appels ONVIF.
    pub async fn set_preset(&self, name: &str) -> Result<String, OnvifError> {
        self.set_preset_at(name, None).await
    }

    /// Comme [`Self::set_preset`], mais ecrase l'emplacement `token` s'il est donne (au lieu d'en prendre un libre).
    ///
    /// # Errors
    /// Erreurs des appels ONVIF.
    pub async fn set_preset_at(
        &self,
        name: &str,
        token: Option<&str>,
    ) -> Result<String, OnvifError> {
        let slot = token
            .map(|t| format!("<tptz:PresetToken>{}</tptz:PresetToken>", xml_escape(t)))
            .unwrap_or_default();
        let xml = self
            .call(&format!(
                "<tptz:SetPreset><tptz:ProfileToken>{}</tptz:ProfileToken><tptz:PresetName>{}</tptz:PresetName>{slot}</tptz:SetPreset>",
                self.token(),
                xml_escape(name)
            ))
            .await?;
        let doc = parse(&xml)?;
        Ok(text(doc.root(), "PresetToken").unwrap_or_default())
    }

    /// # Errors
    /// Erreurs des appels ONVIF.
    pub async fn remove_preset(&self, token: &str) -> Result<(), OnvifError> {
        self.call(&format!(
            "<tptz:RemovePreset><tptz:ProfileToken>{}</tptz:ProfileToken><tptz:PresetToken>{}</tptz:PresetToken></tptz:RemovePreset>",
            self.token(),
            xml_escape(token)
        ))
        .await
        .map(|_| ())
    }

    /// Va a un preset de la camera, a mi-vitesse (comme la reference), puis attend la fin du mouvement.
    ///
    /// # Errors
    /// Erreurs des appels ONVIF.
    pub async fn goto_preset(&self, token: &str) -> Result<(), OnvifError> {
        self.call(&format!(
            r#"<tptz:GotoPreset><tptz:ProfileToken>{}</tptz:ProfileToken><tptz:PresetToken>{}</tptz:PresetToken><tptz:Speed><tt:PanTilt x="0.5" y="0.5"/><tt:Zoom x="0.5"/></tptz:Speed></tptz:GotoPreset>"#,
            self.token(),
            xml_escape(token)
        ))
        .await?;
        self.wait_idle(Duration::from_secs(8)).await;
        Ok(())
    }

    /// Ce que le noeud PTZ annonce (`GetNodes`), en une ligne pour le journal : origine, nombre de presets, espaces de coordonnees.
    /// Sert a comprendre une camera qui accepte des ordres sans les executer.
    ///
    /// # Errors
    /// Erreurs des appels ONVIF.
    pub async fn capabilities(&self) -> Result<String, OnvifError> {
        let xml = self.call("<tptz:GetNodes/>").await?;
        let doc = parse(&xml)?;
        let node = doc
            .root()
            .descendants()
            .find(|d| d.is_element() && d.tag_name().name() == "PTZNode");
        let Some(node) = node else {
            return Ok("no PTZ node announced".into());
        };
        let spaces: Vec<&str> = find(node, "SupportedPTZSpaces")
            .map(|s| {
                s.children()
                    .filter(roxmltree::Node::is_element)
                    .map(|c| c.tag_name().name())
                    .collect()
            })
            .unwrap_or_default();
        Ok(format!(
            "origine={} presets_max={} origine_fixe={} espaces={}",
            text(node, "HomeSupported").unwrap_or_else(|| "?".into()),
            text(node, "MaximumNumberOfPresets").unwrap_or_else(|| "?".into()),
            node.attribute("FixedHomePosition").unwrap_or("?"),
            spaces.join(",")
        ))
    }

    /// Retourne a la position d'origine de la camera (`GotoHomePosition`), puis attend la fin du mouvement.
    ///
    /// # Errors
    /// Erreurs des appels ONVIF (beaucoup de cameras n'ont pas de position d'origine definie).
    pub async fn go_home(&self) -> Result<(), OnvifError> {
        self.call(&format!(
            r#"<tptz:GotoHomePosition><tptz:ProfileToken>{}</tptz:ProfileToken><tptz:Speed><tt:PanTilt x="0.5" y="0.5"/><tt:Zoom x="0.5"/></tptz:Speed></tptz:GotoHomePosition>"#,
            self.token()
        ))
        .await?;
        self.wait_idle(Duration::from_secs(8)).await;
        Ok(())
    }

    /// Va a une position enregistree en base (camera sans preset ONVIF) : `AbsoluteMove` dans l'espace que la camera utilise
    /// (Thingino : degres ; le defaut serait `[-1, 1]`), puis attend la fin du mouvement.
    ///
    /// # Errors
    /// Erreurs des appels ONVIF.
    pub async fn absolute_move(&self, position: &PtzPosition) -> Result<(), OnvifError> {
        let space = position
            .space
            .as_deref()
            .map(|s| format!(r#" space="{}""#, xml_escape(s)))
            .unwrap_or_default();
        self.call(&format!(
            r#"<tptz:AbsoluteMove><tptz:ProfileToken>{}</tptz:ProfileToken><tptz:Position><tt:PanTilt x="{}" y="{}"{space}/><tt:Zoom x="{}"/></tptz:Position><tptz:Speed><tt:PanTilt x="0.5" y="0.5"/><tt:Zoom x="0.5"/></tptz:Speed></tptz:AbsoluteMove>"#,
            self.token(),
            position.pan,
            position.tilt,
            position.zoom
        ))
        .await?;
        self.wait_idle(Duration::from_secs(8)).await;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn speeds_and_axes_are_clamped_like_reference_and_never_nan() {
        assert!((clamp_speed(0.0) - MIN_SPEED).abs() < f64::EPSILON);
        assert!((clamp_speed(3.0) - 1.0).abs() < f64::EPSILON);
        assert!((clamp_speed(f64::NAN) - 0.5).abs() < f64::EPSILON);
        assert!((clamp_axis(-7.0) + 1.0).abs() < f64::EPSILON);
        assert!(clamp_axis(f64::NAN).abs() < f64::EPSILON);
        assert!((clamp_axis(0.3) - 0.3).abs() < f64::EPSILON);
    }
}
