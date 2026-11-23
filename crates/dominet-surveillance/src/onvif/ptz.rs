// SPDX-License-Identifier: AGPL-3.0-or-later
//! ONVIF PTZ (R6 lot 5, ADR 0047): `ContinuousMove` / `Stop` (the only movements of the interface), positions, presets, `AbsoluteMove`.
//!
//! A [`PtzControl`] holds the address of the PTZ service and the profile token: they are read once (`GetCapabilities`, `GetProfiles`)
//! then reused throughout a control session.

use std::fmt::Write;
use std::time::Duration;

use roxmltree::Node;

use super::{OnvifClient, OnvifError, find, parse, text, xml_escape};

/// Minimum accepted speed (like the reference: below it, several cameras do not move).
pub const MIN_SPEED: f64 = 0.05;

/// A PTZ position, in the space the camera announces (`space`: address of the coordinate space, if known).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct PtzPosition {
    pub pan: f64,
    pub tilt: f64,
    pub zoom: f64,
    /// Pan/tilt coordinate space (Thingino: degrees, `SphericalPositionSpace`).
    pub space: Option<String>,
}

/// A PTZ position stored in the camera.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CameraPreset {
    pub token: String,
    pub name: String,
}

/// Is the camera at rest?
#[derive(Debug, Clone, PartialEq, Default)]
pub struct PtzStatus {
    pub position: Option<PtzPosition>,
    /// `true` if `MoveStatus` announces `IDLE` (or if the camera does not say).
    pub idle: bool,
}

/// Brings a speed back into `MIN_SPEED..=1.0` (an unreadable value gives the middle).
#[must_use]
pub fn clamp_speed(speed: f64) -> f64 {
    if speed.is_nan() {
        0.5
    } else {
        speed.clamp(MIN_SPEED, 1.0)
    }
}

/// A speed component in `-1.0..=1.0` (an unreadable value is zero: we do not order an uncertain movement).
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
    /// Prepares PTZ control: address of the PTZ service and first profile that carries a PTZ configuration.
    ///
    /// # Errors
    /// `NoService("PTZ")` if the camera has no PTZ; usual errors of the ONVIF calls.
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

    /// Continuous movement: `pan` and `tilt` in `-1..=1`, `zoom` in `-1..=1` (0 = no movement on that axis).
    ///
    /// # Errors
    /// Errors of the ONVIF calls.
    pub async fn continuous_move(&self, pan: f64, tilt: f64, zoom: f64) -> Result<(), OnvifError> {
        let (pan, tilt, zoom) = (clamp_axis(pan), clamp_axis(tilt), clamp_axis(zoom));
        let mut velocity = String::new();
        // Like the reference: only the moving axes are sent.
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

    /// Stops all movement (pan / tilt and zoom).
    ///
    /// # Errors
    /// Errors of the ONVIF calls.
    pub async fn stop(&self) -> Result<(), OnvifError> {
        self.call(&format!(
            "<tptz:Stop><tptz:ProfileToken>{}</tptz:ProfileToken><tptz:PanTilt>true</tptz:PanTilt><tptz:Zoom>true</tptz:Zoom></tptz:Stop>",
            self.token()
        ))
        .await
        .map(|_| ())
    }

    /// Position and movement state (`None` for the position if the camera does not give it: the NVT always returns 0, 0).
    ///
    /// # Errors
    /// Errors of the ONVIF calls.
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

    /// Waits for a movement to end (at most `timeout`), like the reference after `GotoPreset` / `AbsoluteMove`.
    pub async fn wait_idle(&self, timeout: Duration) {
        let start = tokio::time::Instant::now();
        while start.elapsed() < timeout {
            match self.status().await {
                Ok(s) if !s.idle => tokio::time::sleep(Duration::from_millis(150)).await,
                _ => return,
            }
        }
    }

    /// Presets stored in the camera.
    ///
    /// # Errors
    /// Errors of the ONVIF calls.
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

    /// Saves the current position under this name; returns the token given by the camera (empty if it does not say).
    ///
    /// # Errors
    /// Errors of the ONVIF calls.
    pub async fn set_preset(&self, name: &str) -> Result<String, OnvifError> {
        self.set_preset_at(name, None).await
    }

    /// Like [`Self::set_preset`], but overwrites the `token` slot if given (instead of taking a free one).
    ///
    /// # Errors
    /// Errors of the ONVIF calls.
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
    /// Errors of the ONVIF calls.
    pub async fn remove_preset(&self, token: &str) -> Result<(), OnvifError> {
        self.call(&format!(
            "<tptz:RemovePreset><tptz:ProfileToken>{}</tptz:ProfileToken><tptz:PresetToken>{}</tptz:PresetToken></tptz:RemovePreset>",
            self.token(),
            xml_escape(token)
        ))
        .await
        .map(|_| ())
    }

    /// Goes to a camera preset, at half speed (like the reference), then waits for the movement to end.
    ///
    /// # Errors
    /// Errors of the ONVIF calls.
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

    /// What the PTZ node announces (`GetNodes`), in one line for the log: origin, number of presets, coordinate spaces.
    /// Used to understand a camera that accepts orders without carrying them out.
    ///
    /// # Errors
    /// Errors of the ONVIF calls.
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

    /// Returns to the camera's home position (`GotoHomePosition`), then waits for the movement to end.
    ///
    /// # Errors
    /// Errors of the ONVIF calls (many cameras have no home position defined).
    pub async fn go_home(&self) -> Result<(), OnvifError> {
        self.call(&format!(
            r#"<tptz:GotoHomePosition><tptz:ProfileToken>{}</tptz:ProfileToken><tptz:Speed><tt:PanTilt x="0.5" y="0.5"/><tt:Zoom x="0.5"/></tptz:Speed></tptz:GotoHomePosition>"#,
            self.token()
        ))
        .await?;
        self.wait_idle(Duration::from_secs(8)).await;
        Ok(())
    }

    /// Goes to a position stored in the database (camera without an ONVIF preset): `AbsoluteMove` in the space the camera uses
    /// (Thingino: degrees; the default would be `[-1, 1]`), then waits for the movement to end.
    ///
    /// # Errors
    /// Errors of the ONVIF calls.
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
