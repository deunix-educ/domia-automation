// SPDX-License-Identifier: AGPL-3.0-or-later
//! Imaging ONVIF (R6 lot 5, ADR 0047) : filtre IR-cut (jour / nuit) et luminosite.
//!
//! Comme la reference (et pour la meme raison) : `GetImagingSettings`, on modifie **un seul champ**, on renvoie **l'objet complet** par
//! `SetImagingSettings`. Envoyer un objet partiel remet tous les autres reglages a zero sur la plupart des cameras.

use std::fmt::Write;

use roxmltree::{Document, Node};

use super::{OnvifClient, OnvifError, find, parse, text, xml_escape};

/// Valeur du filtre IR-cut selon ONVIF.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IrCut {
    Day,
    Night,
    Auto,
}

impl IrCut {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Day => "DAY",
            Self::Night => "NIGHT",
            Self::Auto => "AUTO",
        }
    }

    /// Lit la valeur envoyee par la camera (`ON` / `OFF` des anciennes cameras : ON = filtre en place = jour).
    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_uppercase().as_str() {
            "DAY" | "ON" => Some(Self::Day),
            "NIGHT" | "OFF" => Some(Self::Night),
            "AUTO" => Some(Self::Auto),
            _ => None,
        }
    }

    /// La bascule de la reference : `AUTO` et `DAY` vont a `NIGHT`, `NIGHT` revient a `DAY`.
    #[must_use]
    pub fn toggled(self) -> Self {
        match self {
            Self::Night => Self::Day,
            Self::Day | Self::Auto => Self::Night,
        }
    }
}

/// Reglages d'image lus dans la camera, gardes sous leur forme brute pour etre renvoyes en entier.
#[derive(Debug, Clone)]
pub struct ImagingSettings {
    /// Elements fils de `ImagingSettings`, reecrits dans l'espace de noms `tt:` (un par element, sans declaration de prefixe).
    children_xml: String,
    pub ir_cut: Option<IrCut>,
    pub brightness: Option<f64>,
}

/// Recopie un element et ses descendants dans l'espace `tt:` (les prefixes de la camera ne sont pas declares dans notre enveloppe).
fn rewrite(node: Node, out: &mut String) {
    let name = node.tag_name().name();
    let _ = write!(out, "<tt:{name}");
    for a in node.attributes() {
        // Les declarations de prefixes sont des attributs d'espace de noms, que roxmltree ne liste pas ; le reste est conserve.
        let _ = write!(out, " {}=\"{}\"", a.name(), xml_escape(a.value()));
    }
    out.push('>');
    for child in node.children() {
        if child.is_element() {
            rewrite(child, out);
        } else if let Some(t) = child.text() {
            out.push_str(&xml_escape(t));
        }
    }
    let _ = write!(out, "</tt:{name}>");
}

impl ImagingSettings {
    fn from_doc(doc: &Document) -> Result<Self, OnvifError> {
        let settings = find(doc.root(), "ImagingSettings")
            .ok_or(OnvifError::BadResponse("no ImagingSettings"))?;
        let mut children_xml = String::new();
        for child in settings.children().filter(Node::is_element) {
            rewrite(child, &mut children_xml);
        }
        Ok(Self {
            ir_cut: text(settings, "IrCutFilter").and_then(|v| IrCut::parse(&v)),
            brightness: text(settings, "Brightness").and_then(|v| v.parse().ok()),
            children_xml,
        })
    }

    /// Les reglages avec `IrCutFilter` remplace (ajoute s'il manquait).
    fn with_element(&self, name: &str, value: &str) -> String {
        let open = format!("<tt:{name}>");
        let close = format!("</tt:{name}>");
        match (
            self.children_xml.find(&open),
            self.children_xml.find(&close),
        ) {
            (Some(a), Some(b)) if b > a => format!(
                "{}{open}{}{close}{}",
                &self.children_xml[..a],
                xml_escape(value),
                &self.children_xml[b + close.len()..]
            ),
            _ => format!("{}{open}{}{close}", self.children_xml, xml_escape(value)),
        }
    }
}

impl OnvifClient {
    /// Adresse du service d'imagerie et jeton de la source video du profil principal.
    async fn imaging_target(&self) -> Result<(String, String), OnvifError> {
        let services = self.services().await?;
        let url = services.imaging.ok_or(OnvifError::NoService("Imaging"))?;
        let media = services.media.ok_or(OnvifError::NoService("Media"))?;
        let source = self
            .profiles(&media)
            .await?
            .into_iter()
            .find_map(|p| p.source_token)
            .ok_or(OnvifError::BadResponse("no video source"))?;
        Ok((url, source))
    }

    async fn read_imaging(&self, url: &str, source: &str) -> Result<ImagingSettings, OnvifError> {
        let xml = self
            .authed(
                url,
                &format!(
                    "<timg:GetImagingSettings><timg:VideoSourceToken>{}</timg:VideoSourceToken></timg:GetImagingSettings>",
                    xml_escape(source)
                ),
            )
            .await?;
        ImagingSettings::from_doc(&parse(&xml)?)
    }

    async fn write_imaging(
        &self,
        url: &str,
        source: &str,
        settings: &str,
    ) -> Result<(), OnvifError> {
        self.authed(
            url,
            &format!(
                "<timg:SetImagingSettings><timg:VideoSourceToken>{}</timg:VideoSourceToken><timg:ImagingSettings>{settings}</timg:ImagingSettings><timg:ForcePersistence>true</timg:ForcePersistence></timg:SetImagingSettings>",
                xml_escape(source)
            ),
        )
        .await
        .map(|_| ())
    }

    /// Bascule le filtre IR-cut (`AUTO`, `DAY` -> `NIGHT`, `NIGHT` -> `DAY`) ; retourne (avant, apres).
    ///
    /// # Errors
    /// `NoService("Imaging")` sans service d'imagerie, erreurs des appels ONVIF.
    pub async fn toggle_ir_cut(&self) -> Result<(IrCut, IrCut), OnvifError> {
        let (url, source) = self.imaging_target().await?;
        let current = self.read_imaging(&url, &source).await?;
        // Reference : absent -> `DAY`.
        let before = current.ir_cut.unwrap_or(IrCut::Day);
        let after = before.toggled();
        self.write_imaging(
            &url,
            &source,
            &current.with_element("IrCutFilter", after.as_str()),
        )
        .await?;
        Ok((before, after))
    }

    /// Bascule la « lumiere » comme la reference (ONVIF ne standardise pas la LED infrarouge) : luminosite a 80 si elle est sous 40, sinon 0.
    /// Retourne la nouvelle luminosite.
    ///
    /// # Errors
    /// `NoService("Imaging")` sans service d'imagerie, erreurs des appels ONVIF.
    pub async fn toggle_light_brightness(&self) -> Result<f64, OnvifError> {
        let (url, source) = self.imaging_target().await?;
        let current = self.read_imaging(&url, &source).await?;
        // Reference : absent ou nul -> 50 (donc 0 apres bascule).
        let now = current.brightness.filter(|b| *b != 0.0).unwrap_or(50.0);
        let next = if now < 40.0 { 80.0 } else { 0.0 };
        self.write_imaging(
            &url,
            &source,
            &current.with_element("Brightness", &next.to_string()),
        )
        .await?;
        Ok(next)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RESPONSE: &str = r#"<s:Envelope xmlns:s="http://www.w3.org/2003/05/soap-envelope" xmlns:x="http://www.onvif.org/ver10/schema"><s:Body><y:GetImagingSettingsResponse xmlns:y="http://www.onvif.org/ver20/imaging/wsdl"><y:ImagingSettings><x:BacklightCompensation><x:Mode>OFF</x:Mode></x:BacklightCompensation><x:Brightness>35</x:Brightness><x:IrCutFilter>AUTO</x:IrCutFilter><x:Sharpness>50</x:Sharpness></y:ImagingSettings></y:GetImagingSettingsResponse></s:Body></s:Envelope>"#;

    #[test]
    fn the_toggle_follows_reference() {
        assert_eq!(IrCut::Auto.toggled(), IrCut::Night);
        assert_eq!(IrCut::Day.toggled(), IrCut::Night);
        assert_eq!(IrCut::Night.toggled(), IrCut::Day);
        assert_eq!(IrCut::parse("on"), Some(IrCut::Day));
        assert_eq!(IrCut::parse("OFF"), Some(IrCut::Night));
        assert_eq!(IrCut::parse("bizarre"), None);
    }

    #[test]
    fn all_the_other_settings_survive_the_change_of_one_field() {
        let settings = ImagingSettings::from_doc(&Document::parse(RESPONSE).unwrap()).unwrap();
        assert_eq!(settings.ir_cut, Some(IrCut::Auto));
        assert_eq!(settings.brightness, Some(35.0));
        let out = settings.with_element("IrCutFilter", "NIGHT");
        assert_eq!(
            out,
            "<tt:BacklightCompensation><tt:Mode>OFF</tt:Mode></tt:BacklightCompensation><tt:Brightness>35</tt:Brightness><tt:IrCutFilter>NIGHT</tt:IrCutFilter><tt:Sharpness>50</tt:Sharpness>"
        );
        // Un champ absent est ajoute a la fin.
        let bare = ImagingSettings::from_doc(
            &Document::parse(&RESPONSE.replace("<x:IrCutFilter>AUTO</x:IrCutFilter>", "")).unwrap(),
        )
        .unwrap();
        assert!(
            bare.with_element("IrCutFilter", "DAY")
                .ends_with("<tt:IrCutFilter>DAY</tt:IrCutFilter>")
        );
    }
}
