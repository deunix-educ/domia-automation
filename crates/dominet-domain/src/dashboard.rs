// SPDX-License-Identifier: AGPL-3.0-or-later
//! Tableaux de bord et widgets (R2, ADR 0018).
//!
//! Un tableau appartient a un compte ; il porte des widgets places sur une grille de [`GRID_COLUMNS`] colonnes. Les regles
//! (rectangle dans la grille, pas de chevauchement, coherence type / presentation / references, configuration Grafana)
//! vivent ici pour etre appliquees de la meme facon par le serveur et les tests ; la base garantit en plus l'absence de
//! chevauchement (contrainte d'exclusion).

use std::str::FromStr;

use serde_json::Value;
use uuid::Uuid;

use crate::error::{Problem, ValidationError};
use crate::user::UserId;

/// Nombre de colonnes de la grille.
pub const GRID_COLUMNS: i16 = 12;
/// Derniere ligne autorisee.
pub const MAX_ROW: i16 = 99;
/// Largeur et hauteur par defaut d'un widget.
pub const DEFAULT_SPAN: i16 = 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct DashboardId(pub i64);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct WidgetId(pub i64);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Dashboard {
    pub id: DashboardId,
    pub name: String,
    pub owner: UserId,
    /// Le tableau ouvert par defaut (au plus un par proprietaire).
    pub is_default: bool,
}

/// Tableau a creer ou a modifier. `is_default` ne peut qu'**etre active** : le tableau par defaut change quand on en
/// designe un autre (le premier tableau d'un compte est toujours par defaut).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DashboardDraft {
    pub name: String,
    pub is_default: bool,
}

impl DashboardDraft {
    pub const NAME_MAX: usize = 64;

    /// Problemes champ par champ (`name`).
    #[must_use]
    pub fn problems(&self) -> Vec<(&'static str, Problem)> {
        let mut out = vec![];
        if self.name.trim().is_empty() {
            out.push(("name", Problem::Required));
        } else if self.name.chars().count() > Self::NAME_MAX {
            out.push((
                "name",
                Problem::TooLong {
                    max: Self::NAME_MAX,
                },
            ));
        }
        out
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WidgetType {
    Camera,
    Device,
    Scene,
    Grafana,
    Text,
    Grille,
}

impl WidgetType {
    pub const ALL: [Self; 6] = [
        Self::Camera,
        Self::Device,
        Self::Scene,
        Self::Grafana,
        Self::Text,
        Self::Grille,
    ];

    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Camera => "camera",
            Self::Device => "device",
            Self::Scene => "scene",
            Self::Grafana => "grafana",
            Self::Text => "text",
            Self::Grille => "grille",
        }
    }
}

impl FromStr for WidgetType {
    type Err = ValidationError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::ALL
            .into_iter()
            .find(|t| t.as_str() == s)
            .ok_or_else(|| ValidationError(format!("unknown widget type: \"{s}\"")))
    }
}

/// Modele d'affichage d'un widget. `Auto` = defaut selon le type (et, pour un appareil, selon son type).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Presentation {
    Auto,
    DeviceSensor,
    DeviceBinarySensor,
    DeviceSwitch,
    DeviceLight,
    DeviceCover,
    DeviceClimate,
    DeviceFan,
    DeviceLock,
    DeviceGeneric,
    DeviceWeather,
    Camera,
    Grafana,
    Scene,
    Text,
}

impl Presentation {
    pub const ALL: [Self; 15] = [
        Self::Auto,
        Self::DeviceSensor,
        Self::DeviceBinarySensor,
        Self::DeviceSwitch,
        Self::DeviceLight,
        Self::DeviceCover,
        Self::DeviceClimate,
        Self::DeviceFan,
        Self::DeviceLock,
        Self::DeviceGeneric,
        Self::DeviceWeather,
        Self::Camera,
        Self::Grafana,
        Self::Scene,
        Self::Text,
    ];

    /// Valeur stockee (chaine vide pour `Auto`) ; identique a l'implementation de reference.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Auto => "",
            Self::DeviceSensor => "device/sensor",
            Self::DeviceBinarySensor => "device/binary_sensor",
            Self::DeviceSwitch => "device/switch",
            Self::DeviceLight => "device/light",
            Self::DeviceCover => "device/cover",
            Self::DeviceClimate => "device/climate",
            Self::DeviceFan => "device/fan",
            Self::DeviceLock => "device/lock",
            Self::DeviceGeneric => "device/generic",
            Self::DeviceWeather => "device/weather",
            Self::Camera => "camera",
            Self::Grafana => "grafana",
            Self::Scene => "scene",
            Self::Text => "text",
        }
    }

    /// Vrai si cette presentation peut s'appliquer a un widget de ce type.
    #[must_use]
    pub fn fits(self, widget_type: WidgetType) -> bool {
        match self {
            Self::Auto => true,
            Self::Camera => widget_type == WidgetType::Camera,
            Self::Grafana => widget_type == WidgetType::Grafana,
            Self::Scene => widget_type == WidgetType::Scene,
            Self::Text => widget_type == WidgetType::Text,
            _ => widget_type == WidgetType::Device,
        }
    }
}

impl FromStr for Presentation {
    type Err = ValidationError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::ALL
            .into_iter()
            .find(|p| p.as_str() == s)
            .ok_or_else(|| ValidationError(format!("unknown presentation: \"{s}\"")))
    }
}

/// Position et taille d'un widget, en cellules de la grille.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GridRect {
    pub col: i16,
    pub row: i16,
    pub width: i16,
    pub height: i16,
}

impl GridRect {
    /// Rectangle par defaut d'un nouveau widget, a l'origine.
    pub const DEFAULT: Self = Self {
        col: 0,
        row: 0,
        width: DEFAULT_SPAN,
        height: DEFAULT_SPAN,
    };

    /// Problemes champ par champ (`col`, `row`, `width`, `height`) : le rectangle doit tenir dans la grille.
    #[must_use]
    pub fn problems(&self) -> Vec<(&'static str, Problem)> {
        let range = |min: i16, max: i16| Problem::OutOfRange {
            min: i64::from(min),
            max: i64::from(max),
        };
        let mut out = vec![];
        if !(0..GRID_COLUMNS).contains(&self.col) {
            out.push(("col", range(0, GRID_COLUMNS - 1)));
        }
        if !(0..=MAX_ROW).contains(&self.row) {
            out.push(("row", range(0, MAX_ROW)));
        }
        if !(1..=GRID_COLUMNS).contains(&self.height) {
            out.push(("height", range(1, GRID_COLUMNS)));
        }
        if !(1..=GRID_COLUMNS).contains(&self.width) {
            out.push(("width", range(1, GRID_COLUMNS)));
        } else if (0..GRID_COLUMNS).contains(&self.col) && self.col + self.width > GRID_COLUMNS {
            // Largeur valable seule, mais qui deborde a droite a partir de cette colonne.
            out.push(("width", range(1, GRID_COLUMNS - self.col)));
        }
        out
    }

    /// Vrai si les deux rectangles partagent au moins une cellule (des rectangles contigus ne se chevauchent pas).
    #[must_use]
    pub fn overlaps(&self, other: &Self) -> bool {
        self.col < other.col + other.width
            && other.col < self.col + self.width
            && self.row < other.row + other.height
            && other.row < self.row + self.height
    }
}

/// Widget a creer ou a remplacer.
#[derive(Debug, Clone, PartialEq)]
pub struct WidgetDraft {
    pub widget_type: WidgetType,
    pub title: String,
    pub device_id: Option<i64>,
    pub scene_id: Option<i64>,
    pub camera_uuid: Option<Uuid>,
    pub presentation: Presentation,
    /// Objet JSON : Grafana (`panel_url`, ou `dashboard_uid` + `panel_id`, ou `key` avec un appareil), camera (`grafana_*`).
    pub config: Value,
    /// HTML du widget `text` (vide pour les autres types).
    pub content: String,
    pub rect: GridRect,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Widget {
    pub id: WidgetId,
    pub dashboard: DashboardId,
    pub widget_type: WidgetType,
    pub title: String,
    pub device_id: Option<i64>,
    pub scene_id: Option<i64>,
    pub camera_uuid: Option<Uuid>,
    pub presentation: Presentation,
    pub config: Value,
    pub content: String,
    pub rect: GridRect,
}

impl Widget {
    /// La meme donnee sous forme de brouillon (pour la modifier puis la renvoyer).
    #[must_use]
    pub fn to_draft(&self) -> WidgetDraft {
        WidgetDraft {
            widget_type: self.widget_type,
            title: self.title.clone(),
            device_id: self.device_id,
            scene_id: self.scene_id,
            camera_uuid: self.camera_uuid,
            presentation: self.presentation,
            config: self.config.clone(),
            content: self.content.clone(),
            rect: self.rect,
        }
    }
}

fn is_http_url(s: &str) -> bool {
    (s.starts_with("http://") || s.starts_with("https://"))
        && s.len() <= 512
        && !s.contains(char::is_whitespace)
}

fn is_uid(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 64
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

impl WidgetDraft {
    pub const TITLE_MAX: usize = 64;
    pub const CONFIG_MAX_BYTES: usize = 4096;
    pub const CONTENT_MAX: usize = 20_000;

    /// Brouillon d'un type donne, sans reference ni configuration, a l'origine de la grille.
    #[must_use]
    pub fn new(widget_type: WidgetType) -> Self {
        Self {
            widget_type,
            title: String::new(),
            device_id: None,
            scene_id: None,
            camera_uuid: None,
            presentation: Presentation::Auto,
            config: Value::Object(serde_json::Map::new()),
            content: String::new(),
            rect: GridRect::DEFAULT,
        }
    }

    /// Tous les problemes, champ par champ (`title`, `device_id`, `scene_id`, `camera_uuid`, `presentation`, `config`,
    /// `content`, et ceux du rectangle).
    #[must_use]
    pub fn problems(&self) -> Vec<(&'static str, Problem)> {
        let mut out = vec![];
        if self.title.chars().count() > Self::TITLE_MAX {
            out.push((
                "title",
                Problem::TooLong {
                    max: Self::TITLE_MAX,
                },
            ));
        }
        self.reference_problems(&mut out);
        if !self.presentation.fits(self.widget_type) {
            out.push(("presentation", Problem::Invalid));
        }
        self.config_problems(&mut out);
        let content_allowed = self.widget_type == WidgetType::Text;
        if self.content.chars().count() > Self::CONTENT_MAX {
            out.push((
                "content",
                Problem::TooLong {
                    max: Self::CONTENT_MAX,
                },
            ));
        } else if !content_allowed && !self.content.is_empty() {
            out.push(("content", Problem::Invalid));
        }
        out.extend(self.rect.problems());
        out
    }

    /// Un widget `device` designe un appareil, `scene` une scene, `camera` une camera ; Grafana peut en plus s'appuyer sur un
    /// appareil ; `text` et `grille` ne designent rien. Toute autre reference est refusee.
    fn reference_problems(&self, out: &mut Vec<(&'static str, Problem)>) {
        let t = self.widget_type;
        let device_ok = matches!(t, WidgetType::Device | WidgetType::Grafana);
        match (t == WidgetType::Device, self.device_id) {
            (true, None) => out.push(("device_id", Problem::Required)),
            (false, Some(_)) if !device_ok => out.push(("device_id", Problem::Invalid)),
            _ => {}
        }
        match (t == WidgetType::Scene, self.scene_id) {
            (true, None) => out.push(("scene_id", Problem::Required)),
            (false, Some(_)) => out.push(("scene_id", Problem::Invalid)),
            _ => {}
        }
        match (t == WidgetType::Camera, self.camera_uuid) {
            (true, None) => out.push(("camera_uuid", Problem::Required)),
            (false, Some(_)) => out.push(("camera_uuid", Problem::Invalid)),
            _ => {}
        }
        if self.device_id.is_some_and(|id| id <= 0) {
            out.push(("device_id", Problem::Invalid));
        }
        if self.scene_id.is_some_and(|id| id <= 0) {
            out.push(("scene_id", Problem::Invalid));
        }
    }

    fn config_problems(&self, out: &mut Vec<(&'static str, Problem)>) {
        let Some(map) = self.config.as_object() else {
            out.push(("config", Problem::Invalid));
            return;
        };
        if self.config.to_string().len() > Self::CONFIG_MAX_BYTES {
            out.push((
                "config",
                Problem::TooLong {
                    max: Self::CONFIG_MAX_BYTES,
                },
            ));
            return;
        }
        let text = |k: &str| map.get(k).and_then(Value::as_str);
        let bad = |k: &str, ok: fn(&str) -> bool| {
            map.get(k)
                .is_some_and(|v| v.as_str().is_none_or(|s| !ok(s)))
        };
        match self.widget_type {
            WidgetType::Grafana => {
                let by_url = text("panel_url").is_some_and(is_http_url);
                let by_uid = text("dashboard_uid").is_some_and(is_uid)
                    && map.get("panel_id").is_some_and(|v| v.as_u64().is_some());
                let by_device =
                    self.device_id.is_some() && text("key").is_some_and(|k| !k.trim().is_empty());
                if map.is_empty() || !(by_url || by_uid || by_device) {
                    out.push((
                        "config",
                        if map.is_empty() {
                            Problem::Required
                        } else {
                            Problem::Invalid
                        },
                    ));
                } else if bad("panel_url", is_http_url) || bad("dashboard_uid", is_uid) {
                    out.push(("config", Problem::Invalid));
                }
            }
            // Couche Grafana facultative sur le widget camera : verifiee si elle est presente.
            WidgetType::Camera
                if bad("grafana_panel_url", is_http_url) || bad("grafana_uid", is_uid) =>
            {
                out.push(("config", Problem::Invalid));
            }
            _ => {}
        }
    }

    /// # Errors
    /// Le premier probleme, en francais (API, CLI).
    pub fn validate(&self) -> Result<(), ValidationError> {
        match self.problems().first() {
            None => Ok(()),
            Some((field, problem)) => {
                let why = match problem {
                    Problem::Required => "mandatory".to_string(),
                    Problem::TooLong { max } => format!("{max} characters at most"),
                    Problem::Invalid => "invalid value".to_string(),
                    Problem::OutOfRange { min, max } => format!("between {min} and {max}"),
                };
                Err(ValidationError(format!("{field} : {why}")))
            }
        }
    }
}

#[cfg(test)]
// `assert!(x.is_empty())` sur des listes de problemes typees : plus lisible que la comparaison a un tableau vide annote.
#[allow(clippy::assert_is_empty)]
mod tests {
    use serde_json::json;

    use super::*;

    fn p(draft: &WidgetDraft) -> Vec<(&'static str, Problem)> {
        draft.problems()
    }

    #[test]
    fn widget_types_and_presentations_round_trip_with_the_reference_names() {
        for t in WidgetType::ALL {
            assert_eq!(t.as_str().parse::<WidgetType>().unwrap(), t);
        }
        for pres in Presentation::ALL {
            assert_eq!(pres.as_str().parse::<Presentation>().unwrap(), pres);
        }
        assert_eq!(Presentation::Auto.as_str(), "");
        assert_eq!(
            Presentation::DeviceBinarySensor.as_str(),
            "device/binary_sensor"
        );
        assert!("meteo".parse::<WidgetType>().is_err());
        assert!("device/toaster".parse::<Presentation>().is_err());
    }

    #[test]
    fn a_presentation_must_match_the_widget_type() {
        use Presentation::*;
        assert!(Auto.fits(WidgetType::Text));
        assert!(DeviceLight.fits(WidgetType::Device) && !DeviceLight.fits(WidgetType::Scene));
        assert!(Camera.fits(WidgetType::Camera) && !Camera.fits(WidgetType::Device));
        assert!(Text.fits(WidgetType::Text) && !Text.fits(WidgetType::Grille));
        let mut d = WidgetDraft::new(WidgetType::Scene);
        d.scene_id = Some(1);
        d.presentation = DeviceSwitch;
        assert_eq!(p(&d), [("presentation", Problem::Invalid)]);
    }

    #[test]
    fn the_rectangle_must_fit_the_twelve_column_grid() {
        let ok = GridRect {
            col: 10,
            row: 0,
            width: 2,
            height: 2,
        };
        assert!(ok.problems().is_empty());
        assert_eq!(
            GridRect { col: 11, ..ok }.problems(),
            [("width", Problem::OutOfRange { min: 1, max: 1 })],
            "deborde a droite"
        );
        assert_eq!(GridRect { col: 12, ..ok }.problems()[0].0, "col");
        assert_eq!(
            GridRect { row: 100, ..ok }.problems(),
            [("row", Problem::OutOfRange { min: 0, max: 99 })]
        );
        assert_eq!(GridRect { width: 0, ..ok }.problems()[0].0, "width");
        assert_eq!(GridRect { height: 13, ..ok }.problems()[0].0, "height");
        assert_eq!(GridRect { col: -1, ..ok }.problems()[0].0, "col");
        assert!(
            GridRect {
                col: 0,
                row: 0,
                width: 12,
                height: 12
            }
            .problems()
            .is_empty()
        );
    }

    #[test]
    fn rectangles_overlap_only_when_they_share_a_cell() {
        let a = GridRect {
            col: 0,
            row: 0,
            width: 2,
            height: 2,
        };
        assert!(a.overlaps(&GridRect {
            col: 1,
            row: 1,
            width: 2,
            height: 2
        }));
        assert!(a.overlaps(&a));
        assert!(
            !a.overlaps(&GridRect {
                col: 2,
                row: 0,
                width: 2,
                height: 2
            }),
            "contigus a droite"
        );
        assert!(
            !a.overlaps(&GridRect {
                col: 0,
                row: 2,
                width: 2,
                height: 2
            }),
            "contigus dessous"
        );
        assert!(!a.overlaps(&GridRect {
            col: 5,
            row: 5,
            width: 1,
            height: 1
        }));
        assert!(
            GridRect {
                col: 0,
                row: 0,
                width: 12,
                height: 12
            }
            .overlaps(&GridRect {
                col: 11,
                row: 11,
                width: 1,
                height: 1
            })
        );
    }

    #[test]
    fn each_type_needs_its_own_reference() {
        let mut d = WidgetDraft::new(WidgetType::Device);
        assert_eq!(p(&d), [("device_id", Problem::Required)]);
        d.device_id = Some(42);
        assert!(p(&d).is_empty());
        d.device_id = Some(0);
        assert_eq!(p(&d), [("device_id", Problem::Invalid)]);

        let mut s = WidgetDraft::new(WidgetType::Scene);
        assert_eq!(p(&s), [("scene_id", Problem::Required)]);
        s.scene_id = Some(3);
        s.device_id = Some(1);
        assert_eq!(
            p(&s),
            [("device_id", Problem::Invalid)],
            "une scene ne designe pas d'appareil"
        );

        let mut c = WidgetDraft::new(WidgetType::Camera);
        assert_eq!(p(&c), [("camera_uuid", Problem::Required)]);
        c.camera_uuid = Some(Uuid::nil());
        assert!(p(&c).is_empty());

        for t in [WidgetType::Text, WidgetType::Grille] {
            let mut x = WidgetDraft::new(t);
            assert!(p(&x).is_empty());
            x.camera_uuid = Some(Uuid::nil());
            x.scene_id = Some(1);
            x.device_id = Some(1);
            assert_eq!(p(&x).len(), 3, "{t:?} ne designe rien");
        }
    }

    #[test]
    fn a_grafana_widget_needs_one_of_its_three_configurations() {
        let mut g = WidgetDraft::new(WidgetType::Grafana);
        assert_eq!(p(&g), [("config", Problem::Required)]);
        g.config = json!({"panel_url": "https://grafana.example/d-solo/abc/x?panelId=1"});
        assert!(p(&g).is_empty());
        g.config = json!({"dashboard_uid": "dominet-energy", "panel_id": 4});
        assert!(p(&g).is_empty());
        g.config = json!({"key": "temperature"});
        assert_eq!(p(&g), [("config", Problem::Invalid)], "sans appareil");
        g.device_id = Some(7);
        assert!(p(&g).is_empty(), "cle + appareil");
        for bad in [
            json!({"panel_url": "javascript:alert(1)"}),
            json!({"panel_url": "ftp://x"}),
            json!({"panel_url": "https://a b"}),
            json!({"dashboard_uid": "a b", "panel_id": 1}),
            json!({"dashboard_uid": "ok"}),
            json!({"unknown": true}),
        ] {
            let mut x = WidgetDraft::new(WidgetType::Grafana);
            x.config = bad.clone();
            assert!(!p(&x).is_empty(), "{bad}");
        }
    }

    #[test]
    fn the_camera_grafana_layer_is_optional_but_checked() {
        let mut c = WidgetDraft::new(WidgetType::Camera);
        c.camera_uuid = Some(Uuid::nil());
        c.config = json!({"grafana_uid": "dominet-surveillance", "grafana_vars": "var-x=1"});
        assert!(p(&c).is_empty());
        c.config = json!({"grafana_panel_url": "javascript:alert(1)"});
        assert_eq!(p(&c), [("config", Problem::Invalid)]);
    }

    #[test]
    fn config_must_be_a_small_object_and_content_belongs_to_text_widgets() {
        let mut t = WidgetDraft::new(WidgetType::Text);
        t.config = json!([1, 2]);
        assert_eq!(p(&t), [("config", Problem::Invalid)]);
        t.config = json!({"big": "x".repeat(5000)});
        assert_eq!(p(&t), [("config", Problem::TooLong { max: 4096 })]);
        t.config = json!({});
        t.content = "<p>Bonjour</p>".into();
        assert!(p(&t).is_empty());
        t.content = "x".repeat(20_001);
        assert_eq!(p(&t), [("content", Problem::TooLong { max: 20_000 })]);
        let mut g = WidgetDraft::new(WidgetType::Grille);
        g.content = "<p>x</p>".into();
        assert_eq!(p(&g), [("content", Problem::Invalid)]);
        g.content.clear();
        g.title = "t".repeat(65);
        assert_eq!(p(&g), [("title", Problem::TooLong { max: 64 })]);
    }

    #[test]
    fn dashboard_names_are_required_and_bounded() {
        let ok = DashboardDraft {
            name: "Salon".into(),
            is_default: false,
        };
        assert!(ok.problems().is_empty());
        assert_eq!(
            DashboardDraft {
                name: "  ".into(),
                ..ok.clone()
            }
            .problems(),
            [("name", Problem::Required)]
        );
        assert_eq!(
            DashboardDraft {
                name: "x".repeat(65),
                ..ok
            }
            .problems(),
            [("name", Problem::TooLong { max: 64 })]
        );
    }

    #[test]
    fn validate_reports_the_first_problem() {
        let mut d = WidgetDraft::new(WidgetType::Device);
        assert!(d.validate().unwrap_err().0.starts_with("device_id"));
        d.device_id = Some(1);
        assert!(d.validate().is_ok());
    }
}
