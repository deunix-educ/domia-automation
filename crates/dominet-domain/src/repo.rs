// SPDX-License-Identifier: AGPL-3.0-or-later
//! Ports de stockage : les adaptateurs (`dominet-store`) les implementent, le serveur les consomme.
//!
//! Les methodes renvoient `impl Future + Send` pour rester utilisables depuis des taches Tokio.

use std::future::Future;

use chrono::{DateTime, Utc};

use crate::alert::{Alert, AlertId, AlertRule, AlertRuleDraft, AlertRuleId, AlertStatus, Severity};
use crate::android::{
    AndroidDevice, AndroidDeviceDraft, AndroidDeviceId, CallId, CallLog, CallPlan, NewOutbox,
    NewSms, OutboxId, OutboxStatus, SmsId, SmsMessage, SmsOutbox,
};
use crate::auth::AttemptStats;
use crate::automation::{Automation, AutomationDraft, AutomationId, AutomationRun};
use crate::camera::{
    Camera, CameraDraft, CameraId, CameraUuid, Detection, DetectionEvent, DetectionEventId,
    DetectionTrigger, DetectorConfig, MotionEvent, NewRecording, OnvifInfo, PtzPreset,
    PtzPresetDraft, PtzPresetId, Recording, RecordingId, RecordingMode, Snapshot, SnapshotId,
};
use crate::dashboard::{Dashboard, DashboardDraft, DashboardId, Widget, WidgetDraft, WidgetId};
use crate::device::{
    Device, DeviceDraft, DeviceGroup, DeviceGroupDraft, DeviceGroupId, DeviceId, DeviceImage,
    DeviceLabel, DeviceLabelDraft, DeviceLabelId, DeviceType, Discovered, LogEntry, LogPoint,
    MqttSource, MqttSourceDraft, MqttSourceId, WeatherStation, WeatherStationDraft,
    WeatherStationId,
};
use crate::energy::EnergyDay;
use crate::error::RepoError;
use crate::location::{Location, LocationDraft, LocationId};
use crate::mcp::{AuditKind, McpAuditEntry, McpToken, McpTokenId, NewAuditEntry, NewMcpToken};
use crate::messaging::{Delivery, DeliveryId, DeliveryStatus, Message, MessageId, MessageStatus};
use crate::navbar::{NavbarDraft, NavbarItem, NavbarItemId};
use crate::observability::{
    GrafanaRule, GrafanaRuleDraft, GrafanaRuleId, PrometheusRule, PrometheusRuleDraft,
    PrometheusRuleId,
};
use crate::scene::{Scene, SceneDraft, SceneId};
use crate::schedule::{Job, JobDraft, JobId};
use crate::setting::{AppSetting, SettingDraft};
use crate::user::{Credentials, NewUser, User, UserId, UserProfile};

pub type RepoResult<T> = Result<T, RepoError>;

pub trait UserRepo {
    /// Cree un compte. `Conflict` si le nom existe deja (sans tenir compte de la casse).
    fn create(&self, new: &NewUser) -> impl Future<Output = RepoResult<User>> + Send;
    fn find_by_id(&self, id: UserId) -> impl Future<Output = RepoResult<Option<User>>> + Send;
    /// Recherche insensible a la casse.
    fn find_by_username(
        &self,
        username: &str,
    ) -> impl Future<Output = RepoResult<Option<User>>> + Send;
    /// Compte + condensat du mot de passe (connexion).
    fn find_credentials(
        &self,
        username: &str,
    ) -> impl Future<Output = RepoResult<Option<Credentials>>> + Send;
    fn list(&self) -> impl Future<Output = RepoResult<Vec<User>>> + Send;
    fn update_profile(
        &self,
        id: UserId,
        profile: &UserProfile,
    ) -> impl Future<Output = RepoResult<User>> + Send;
    /// Remplace le condensat et date le changement (invalide les anciennes sessions cote serveur).
    fn set_password_hash(
        &self,
        id: UserId,
        hash: &str,
    ) -> impl Future<Output = RepoResult<()>> + Send;
    fn touch_last_login(&self, id: UserId) -> impl Future<Output = RepoResult<()>> + Send;
    fn delete(&self, id: UserId) -> impl Future<Output = RepoResult<()>> + Send;
    /// Nombre de comptes administrateurs actifs (pour interdire de retirer le dernier).
    fn count_active_admins(&self) -> impl Future<Output = RepoResult<i64>> + Send;
}

pub trait SettingRepo {
    /// Tous les reglages, tries par categorie puis cle.
    fn list(&self) -> impl Future<Output = RepoResult<Vec<AppSetting>>> + Send;
    fn get(&self, key: &str) -> impl Future<Output = RepoResult<Option<AppSetting>>> + Send;
    /// Cree le reglage ou remplace sa valeur, son type, sa categorie et sa description.
    /// `stored_value` est la valeur deja prete a stocker (chiffree si `draft.is_secret`).
    fn upsert(
        &self,
        draft: &SettingDraft,
        stored_value: &str,
    ) -> impl Future<Output = RepoResult<AppSetting>> + Send;
    /// Cree le reglage seulement s'il n'existe pas ; renvoie `true` si cree (donnees initiales).
    fn insert_if_absent(
        &self,
        draft: &SettingDraft,
        stored_value: &str,
    ) -> impl Future<Output = RepoResult<bool>> + Send;
    fn delete(&self, key: &str) -> impl Future<Output = RepoResult<()>> + Send;
}

pub trait NavbarRepo {
    /// Entrees triees : haut puis bas, par ordre puis libelle.
    fn list(&self) -> impl Future<Output = RepoResult<Vec<NavbarItem>>> + Send;
    fn get(&self, id: NavbarItemId) -> impl Future<Output = RepoResult<Option<NavbarItem>>> + Send;
    fn create(&self, draft: &NavbarDraft) -> impl Future<Output = RepoResult<NavbarItem>> + Send;
    fn update(
        &self,
        id: NavbarItemId,
        draft: &NavbarDraft,
    ) -> impl Future<Output = RepoResult<NavbarItem>> + Send;
    fn delete(&self, id: NavbarItemId) -> impl Future<Output = RepoResult<()>> + Send;
    /// Cree l'entree si aucune n'a deja la meme position et le meme libelle ; `true` si creee.
    fn insert_if_absent(
        &self,
        draft: &NavbarDraft,
    ) -> impl Future<Output = RepoResult<bool>> + Send;
}

pub trait LocationRepo {
    fn list(&self) -> impl Future<Output = RepoResult<Vec<Location>>> + Send;
    fn get(&self, id: LocationId) -> impl Future<Output = RepoResult<Option<Location>>> + Send;
    fn create(&self, draft: &LocationDraft) -> impl Future<Output = RepoResult<Location>> + Send;
    fn update(
        &self,
        id: LocationId,
        draft: &LocationDraft,
    ) -> impl Future<Output = RepoResult<Location>> + Send;
    fn delete(&self, id: LocationId) -> impl Future<Output = RepoResult<()>> + Send;
}

/// Regles d'alerte Prometheus (R7 lot 2). `list` rend l'ordre du fichier : groupe puis nom.
pub trait PrometheusRuleRepo {
    fn list(&self) -> impl Future<Output = RepoResult<Vec<PrometheusRule>>> + Send;
    fn get(
        &self,
        id: PrometheusRuleId,
    ) -> impl Future<Output = RepoResult<Option<PrometheusRule>>> + Send;
    fn create(
        &self,
        draft: &PrometheusRuleDraft,
    ) -> impl Future<Output = RepoResult<PrometheusRule>> + Send;
    fn update(
        &self,
        id: PrometheusRuleId,
        draft: &PrometheusRuleDraft,
    ) -> impl Future<Output = RepoResult<PrometheusRule>> + Send;
    fn delete(&self, id: PrometheusRuleId) -> impl Future<Output = RepoResult<()>> + Send;
    /// Cree la regle si aucune n'a ce nom ; `true` si creee (graine des regles livrees).
    fn insert_if_absent(
        &self,
        draft: &PrometheusRuleDraft,
    ) -> impl Future<Output = RepoResult<bool>> + Send;
}

/// Regles d'alerte Grafana (R7 lot 2). `list` rend l'ordre du fichier : dossier, groupe puis titre.
pub trait GrafanaRuleRepo {
    fn list(&self) -> impl Future<Output = RepoResult<Vec<GrafanaRule>>> + Send;
    fn get(
        &self,
        id: GrafanaRuleId,
    ) -> impl Future<Output = RepoResult<Option<GrafanaRule>>> + Send;
    /// Cree la regle ; un `uid` vide est tire du titre et rendu unique (`titre`, `titre-1`, ...).
    fn create(
        &self,
        draft: &GrafanaRuleDraft,
    ) -> impl Future<Output = RepoResult<GrafanaRule>> + Send;
    fn update(
        &self,
        id: GrafanaRuleId,
        draft: &GrafanaRuleDraft,
    ) -> impl Future<Output = RepoResult<GrafanaRule>> + Send;
    fn delete(&self, id: GrafanaRuleId) -> impl Future<Output = RepoResult<()>> + Send;
    /// Cree la regle si aucune n'a cet `uid` ; `true` si creee.
    fn insert_if_absent(
        &self,
        draft: &GrafanaRuleDraft,
    ) -> impl Future<Output = RepoResult<bool>> + Send;
}

/// Journal des tentatives de connexion (blocage anti brute-force).
/// Tableaux de bord. Le premier tableau d'un compte est toujours son tableau par defaut ; designer un tableau comme
/// defaut retire ce statut aux autres du meme compte, dans la meme transaction.
pub trait DashboardRepo {
    /// Tableaux d'un compte, par nom.
    fn list_for_owner(
        &self,
        owner: UserId,
    ) -> impl Future<Output = RepoResult<Vec<Dashboard>>> + Send;
    /// Tous les tableaux (administration), par proprietaire puis par nom.
    fn list_all(&self) -> impl Future<Output = RepoResult<Vec<Dashboard>>> + Send;
    fn get(&self, id: DashboardId) -> impl Future<Output = RepoResult<Option<Dashboard>>> + Send;
    /// `NotFound` si le proprietaire n'existe pas.
    fn create(
        &self,
        owner: UserId,
        draft: &DashboardDraft,
    ) -> impl Future<Output = RepoResult<Dashboard>> + Send;
    /// Renomme ; `draft.is_default` ne peut qu'activer le statut (un tableau par defaut le reste jusqu'a ce qu'un autre le devienne).
    fn update(
        &self,
        id: DashboardId,
        draft: &DashboardDraft,
    ) -> impl Future<Output = RepoResult<Dashboard>> + Send;
    /// Supprime le tableau et ses widgets.
    fn delete(&self, id: DashboardId) -> impl Future<Output = RepoResult<()>> + Send;
}

/// Widgets d'un tableau. `Conflict { constraint: "widget_no_overlap" }` si le rectangle chevauche un autre widget du tableau.
pub trait WidgetRepo {
    /// Widgets d'un tableau, par ligne puis colonne.
    fn list(&self, dashboard: DashboardId) -> impl Future<Output = RepoResult<Vec<Widget>>> + Send;
    fn get(&self, id: WidgetId) -> impl Future<Output = RepoResult<Option<Widget>>> + Send;
    /// `NotFound` si le tableau n'existe pas ; `Conflict` si le rectangle chevauche un autre widget. Les ecritures de widgets
    /// d'un meme tableau sont serialisees (verrou du tableau).
    fn create(
        &self,
        dashboard: DashboardId,
        draft: &WidgetDraft,
    ) -> impl Future<Output = RepoResult<Widget>> + Send;
    fn update(
        &self,
        id: WidgetId,
        draft: &WidgetDraft,
    ) -> impl Future<Output = RepoResult<Widget>> + Send;
    /// Deplace le widget (position seulement, taille inchangee).
    fn move_to(
        &self,
        id: WidgetId,
        col: i16,
        row: i16,
    ) -> impl Future<Output = RepoResult<Widget>> + Send;
    fn delete(&self, id: WidgetId) -> impl Future<Output = RepoResult<()>> + Send;
}

pub trait LoginAttemptRepo {
    /// Enregistre une tentative ; le nom est range en minuscules.
    fn record(
        &self,
        username: &str,
        ip: &str,
        succeeded: bool,
    ) -> impl Future<Output = RepoResult<()>> + Send;
    /// Echecs depuis `since` (voir `AttemptStats` pour les trois compteurs).
    fn stats(
        &self,
        username: &str,
        ip: &str,
        since: DateTime<Utc>,
    ) -> impl Future<Output = RepoResult<AttemptStats>> + Send;
    /// Supprime les tentatives plus anciennes que `before` ; renvoie le nombre de lignes supprimees.
    fn purge_before(&self, before: DateTime<Utc>) -> impl Future<Output = RepoResult<u64>> + Send;
}

/// Criteres de la liste des appareils.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DeviceFilter {
    pub group: Option<DeviceGroupId>,
    pub location: Option<LocationId>,
    pub device_type: Option<DeviceType>,
    /// Ne garder que les appareils actifs.
    pub active_only: bool,
}

/// Appareils. Les operations `apply_state`, `set_available`, `upsert_discovered`, `upsert_auto` et `touch_by_topic` sont celles de
/// l'ecoute MQTT : elles reprennent la semantique de l'implementation de reference (voir `device.rs`) et sont **idempotentes**
/// (les doublons QoS 1 ne changent rien d'autre que `last_seen`).
pub trait DeviceRepo {
    /// Appareils par nom.
    fn list(&self, filter: &DeviceFilter) -> impl Future<Output = RepoResult<Vec<Device>>> + Send;
    fn get(&self, id: DeviceId) -> impl Future<Output = RepoResult<Option<Device>>> + Send;
    fn find_by_name(
        &self,
        friendly_name: &str,
    ) -> impl Future<Output = RepoResult<Option<Device>>> + Send;
    /// `Conflict` si le nom existe deja.
    fn create(&self, draft: &DeviceDraft) -> impl Future<Output = RepoResult<Device>> + Send;
    /// Modifie les parametres ; l'etat, la disponibilite et la decouverte ne changent pas.
    fn update(
        &self,
        id: DeviceId,
        draft: &DeviceDraft,
    ) -> impl Future<Output = RepoResult<Device>> + Send;
    fn delete(&self, id: DeviceId) -> impl Future<Output = RepoResult<()>> + Send;
    /// `bridge/devices` : cree l'appareil (type deduit, prefixe `base_topic`, cle masquee `update`) ou met a jour sa description.
    /// Renvoie l'appareil et `true` s'il vient d'etre cree.
    fn upsert_discovered(
        &self,
        d: &Discovered,
        base_topic: &str,
    ) -> impl Future<Output = RepoResult<(Device, bool)>> + Send;
    /// Message d'etat : **remplace** l'etat, `last_seen = maintenant`, `available = true` ; seulement si l'appareil est actif.
    /// `None` si aucun appareil actif de ce nom.
    fn apply_state(
        &self,
        friendly_name: &str,
        state: &serde_json::Value,
    ) -> impl Future<Output = RepoResult<Option<Device>>> + Send;
    /// Message de disponibilite (sans effet sur `last_seen`) ; `None` si le nom est inconnu.
    fn set_available(
        &self,
        friendly_name: &str,
        available: bool,
    ) -> impl Future<Output = RepoResult<Option<Device>>> + Send;
    /// Source MQTT en `auto_device` : cree ou met a jour (et **reactive**) l'appareil, remplace son etat.
    fn upsert_auto(
        &self,
        friendly_name: &str,
        base_topic: &str,
        device_type: DeviceType,
        state: &serde_json::Value,
    ) -> impl Future<Output = RepoResult<(Device, bool)>> + Send;
    /// Marque vus (disponibles, `last_seen`) les appareils actifs dont `{base_topic}/{nom}` ou `base_topic` egale le topic.
    fn touch_by_topic(&self, topic: &str)
    -> impl Future<Output = RepoResult<Vec<DeviceId>>> + Send;
}

pub trait DeviceGroupRepo {
    fn list(&self) -> impl Future<Output = RepoResult<Vec<DeviceGroup>>> + Send;
    fn get(
        &self,
        id: DeviceGroupId,
    ) -> impl Future<Output = RepoResult<Option<DeviceGroup>>> + Send;
    fn create(&self, d: &DeviceGroupDraft) -> impl Future<Output = RepoResult<DeviceGroup>> + Send;
    fn update(
        &self,
        id: DeviceGroupId,
        d: &DeviceGroupDraft,
    ) -> impl Future<Output = RepoResult<DeviceGroup>> + Send;
    /// Les appareils du groupe restent, sans groupe.
    fn delete(&self, id: DeviceGroupId) -> impl Future<Output = RepoResult<()>> + Send;
}

pub trait DeviceLabelRepo {
    /// Par cle.
    fn list(&self) -> impl Future<Output = RepoResult<Vec<DeviceLabel>>> + Send;
    fn get(
        &self,
        id: DeviceLabelId,
    ) -> impl Future<Output = RepoResult<Option<DeviceLabel>>> + Send;
    /// `Conflict` si la cle existe deja.
    fn create(&self, d: &DeviceLabelDraft) -> impl Future<Output = RepoResult<DeviceLabel>> + Send;
    fn update(
        &self,
        id: DeviceLabelId,
        d: &DeviceLabelDraft,
    ) -> impl Future<Output = RepoResult<DeviceLabel>> + Send;
    fn delete(&self, id: DeviceLabelId) -> impl Future<Output = RepoResult<()>> + Send;
    /// Cree les libelles dont la cle n'existe pas encore ; renvoie le nombre cree.
    fn create_missing(
        &self,
        drafts: &[DeviceLabelDraft],
    ) -> impl Future<Output = RepoResult<u64>> + Send;
    /// Toutes les cles presentes dans l'etat d'au moins un appareil (pour `seed-labels`).
    fn state_keys(&self) -> impl Future<Output = RepoResult<Vec<String>>> + Send;
}

/// Historique des cles suivies.
pub trait StateLogRepo {
    /// Ecrit les lignes a l'instant `ts` (a la seconde) ; une ligne existante du meme (appareil, cle, instant) est **remplacee**.
    /// Renvoie le nombre de lignes ecrites.
    fn record(
        &self,
        device: DeviceId,
        ts: DateTime<Utc>,
        entries: &[LogEntry],
    ) -> impl Future<Output = RepoResult<u64>> + Send;
    /// Points d'une cle depuis `since`, du plus ancien au plus recent, au plus `limit`.
    fn points(
        &self,
        device: DeviceId,
        key: &str,
        since: DateTime<Utc>,
        limit: i64,
    ) -> impl Future<Output = RepoResult<Vec<LogPoint>>> + Send;
    /// Plus petite et plus grande valeur **numerique** d'une cle sur `[from, to[` (les valeurs texte sont ignorees) ; `None` sans
    /// aucune valeur. C'est ce qu'agrege le bilan d'energie.
    fn min_max(
        &self,
        device: DeviceId,
        key: &str,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
    ) -> impl Future<Output = RepoResult<(Option<f64>, Option<f64>)>> + Send;
    /// Supprime les lignes anterieures a `before` ; renvoie le nombre supprime.
    fn purge_before(&self, before: DateTime<Utc>) -> impl Future<Output = RepoResult<u64>> + Send;
}

pub trait DeviceImageRepo {
    /// `NotFound` si l'appareil n'existe pas.
    fn put(
        &self,
        device: DeviceId,
        content_type: &str,
        data: &[u8],
    ) -> impl Future<Output = RepoResult<()>> + Send;
    fn get(&self, device: DeviceId)
    -> impl Future<Output = RepoResult<Option<DeviceImage>>> + Send;
    fn delete(&self, device: DeviceId) -> impl Future<Output = RepoResult<()>> + Send;
    /// Appareils qui ont une image, avec sa date de mise a jour (sans lire les images).
    fn with_images(
        &self,
    ) -> impl Future<Output = RepoResult<Vec<(DeviceId, DateTime<Utc>)>>> + Send;
}

pub trait MqttSourceRepo {
    fn list(&self) -> impl Future<Output = RepoResult<Vec<MqttSource>>> + Send;
    fn list_active(&self) -> impl Future<Output = RepoResult<Vec<MqttSource>>> + Send;
    fn get(&self, id: MqttSourceId) -> impl Future<Output = RepoResult<Option<MqttSource>>> + Send;
    fn create(&self, d: &MqttSourceDraft) -> impl Future<Output = RepoResult<MqttSource>> + Send;
    fn update(
        &self,
        id: MqttSourceId,
        d: &MqttSourceDraft,
    ) -> impl Future<Output = RepoResult<MqttSource>> + Send;
    fn delete(&self, id: MqttSourceId) -> impl Future<Output = RepoResult<()>> + Send;
}

pub trait WeatherRepo {
    fn list(&self) -> impl Future<Output = RepoResult<Vec<WeatherStation>>> + Send;
    fn list_enabled(&self) -> impl Future<Output = RepoResult<Vec<WeatherStation>>> + Send;
    fn get(
        &self,
        id: WeatherStationId,
    ) -> impl Future<Output = RepoResult<Option<WeatherStation>>> + Send;
    fn create(
        &self,
        d: &WeatherStationDraft,
    ) -> impl Future<Output = RepoResult<WeatherStation>> + Send;
    fn update(
        &self,
        id: WeatherStationId,
        d: &WeatherStationDraft,
    ) -> impl Future<Output = RepoResult<WeatherStation>> + Send;
    fn delete(&self, id: WeatherStationId) -> impl Future<Output = RepoResult<()>> + Send;
}

// ------------------------------------------------------------------ R4 : scenes, automations, alertes, energie

pub trait SceneRepo {
    /// Par nom, avec leurs actions (dans l'ordre de publication).
    fn list(&self) -> impl Future<Output = RepoResult<Vec<Scene>>> + Send;
    fn get(&self, id: SceneId) -> impl Future<Output = RepoResult<Option<Scene>>> + Send;
    fn find_by_name(&self, name: &str) -> impl Future<Output = RepoResult<Option<Scene>>> + Send;
    /// `Conflict` si le nom existe deja ou si un appareil n'existe pas.
    fn create(&self, d: &SceneDraft) -> impl Future<Output = RepoResult<Scene>> + Send;
    /// Remplace aussi les actions, dans une seule transaction.
    fn update(&self, id: SceneId, d: &SceneDraft)
    -> impl Future<Output = RepoResult<Scene>> + Send;
    /// Les widgets qui la visaient perdent leur scene (`ON DELETE SET NULL`).
    fn delete(&self, id: SceneId) -> impl Future<Output = RepoResult<()>> + Send;
}

pub trait AutomationRepo {
    /// Par nom.
    fn list(&self) -> impl Future<Output = RepoResult<Vec<Automation>>> + Send;
    fn list_enabled(&self) -> impl Future<Output = RepoResult<Vec<Automation>>> + Send;
    fn get(&self, id: AutomationId) -> impl Future<Output = RepoResult<Option<Automation>>> + Send;
    fn create(&self, d: &AutomationDraft) -> impl Future<Output = RepoResult<Automation>> + Send;
    fn update(
        &self,
        id: AutomationId,
        d: &AutomationDraft,
    ) -> impl Future<Output = RepoResult<Automation>> + Send;
    fn delete(&self, id: AutomationId) -> impl Future<Output = RepoResult<()>> + Send;
    /// Un declenchement de plus : `trigger_count + 1` et `last_triggered = at`.
    fn record_trigger(
        &self,
        id: AutomationId,
        at: DateTime<Utc>,
    ) -> impl Future<Output = RepoResult<()>> + Send;
}

/// Journal d'execution des automations.
pub trait AutomationRunRepo {
    fn record(&self, run: &AutomationRun) -> impl Future<Output = RepoResult<()>> + Send;
    /// Les plus recentes d'abord.
    fn recent(
        &self,
        automation: AutomationId,
        limit: i64,
    ) -> impl Future<Output = RepoResult<Vec<AutomationRun>>> + Send;
    /// Toutes automations confondues, les plus recentes d'abord.
    fn recent_all(&self, limit: i64)
    -> impl Future<Output = RepoResult<Vec<AutomationRun>>> + Send;
    fn purge_before(&self, before: DateTime<Utc>) -> impl Future<Output = RepoResult<u64>> + Send;
}

/// Travaux du planificateur.
pub trait ScheduleRepo {
    /// Tous, par nom.
    fn list(&self) -> impl Future<Output = RepoResult<Vec<Job>>> + Send;
    /// Cree le travail de ce nom ou le remplace. Si l'horaire, la tache, ses arguments, le fuseau ou l'activation changent,
    /// le prochain passage est oublie (a recalculer) ; sinon il est conserve.
    fn upsert(&self, d: &JobDraft) -> impl Future<Output = RepoResult<Job>> + Send;
    /// Supprime les travaux **d'automations** qui ne sont pas dans `keep` ; renvoie leur nombre.
    fn prune_automation_jobs(
        &self,
        keep: &[AutomationId],
    ) -> impl Future<Output = RepoResult<u64>> + Send;
    /// Memorise le prochain passage (`None` = a recalculer).
    fn set_next(
        &self,
        id: JobId,
        next: Option<DateTime<Utc>>,
    ) -> impl Future<Output = RepoResult<()>> + Send;
    /// Fin d'un passage : `ran_at` si la tache a tourne (pas pour un passage manque), son statut, le passage suivant.
    fn record(
        &self,
        id: JobId,
        ran_at: Option<DateTime<Utc>>,
        status: &str,
        next: Option<DateTime<Utc>>,
    ) -> impl Future<Output = RepoResult<()>> + Send;
    /// Prochain passage des travaux d'automations actifs.
    fn automation_next_runs(
        &self,
    ) -> impl Future<Output = RepoResult<Vec<(AutomationId, Option<DateTime<Utc>>)>>> + Send;
}

pub trait AlertRuleRepo {
    /// Par appareil puis par nom.
    fn list(&self) -> impl Future<Output = RepoResult<Vec<AlertRule>>> + Send;
    /// Les regles **activees** d'un appareil (evaluees a chaque etat).
    fn list_enabled_for_device(
        &self,
        device: DeviceId,
    ) -> impl Future<Output = RepoResult<Vec<AlertRule>>> + Send;
    fn get(&self, id: AlertRuleId) -> impl Future<Output = RepoResult<Option<AlertRule>>> + Send;
    /// `Conflict` si l'appareil n'existe pas.
    fn create(&self, d: &AlertRuleDraft) -> impl Future<Output = RepoResult<AlertRule>> + Send;
    fn update(
        &self,
        id: AlertRuleId,
        d: &AlertRuleDraft,
    ) -> impl Future<Output = RepoResult<AlertRule>> + Send;
    /// Les alertes deja creees restent, sans regle.
    fn delete(&self, id: AlertRuleId) -> impl Future<Output = RepoResult<()>> + Send;
}

/// Une alerte a creer.
#[derive(Debug, Clone, PartialEq)]
pub struct NewAlert {
    pub rule: AlertRuleId,
    pub device: DeviceId,
    pub severity: Severity,
    pub value_triggered: String,
    pub fired_at: DateTime<Utc>,
}

pub trait AlertRepo {
    /// Les plus recentes d'abord ; `None` = tous les statuts.
    fn list(
        &self,
        status: Option<AlertStatus>,
        limit: i64,
    ) -> impl Future<Output = RepoResult<Vec<Alert>>> + Send;
    /// L'alerte **en cours** la plus recente de la regle pour cet appareil (les acquittees et resolues n'y sont pas).
    fn latest_firing(
        &self,
        rule: AlertRuleId,
        device: DeviceId,
    ) -> impl Future<Output = RepoResult<Option<Alert>>> + Send;
    fn create(&self, new: &NewAlert) -> impl Future<Output = RepoResult<Alert>> + Send;
    /// Resout les alertes en cours de la regle pour cet appareil ; renvoie leur nombre.
    fn resolve_firing(
        &self,
        rule: AlertRuleId,
        device: DeviceId,
        at: DateTime<Utc>,
    ) -> impl Future<Output = RepoResult<u64>> + Send;
    /// Acquitte une alerte **en cours** ; `None` si elle n'existe pas ou n'est plus en cours.
    fn acknowledge(
        &self,
        id: AlertId,
        by: UserId,
        at: DateTime<Utc>,
    ) -> impl Future<Output = RepoResult<Option<Alert>>> + Send;
    fn acknowledge_all(
        &self,
        by: UserId,
        at: DateTime<Utc>,
    ) -> impl Future<Output = RepoResult<u64>> + Send;
    fn count_firing(&self) -> impl Future<Output = RepoResult<i64>> + Send;
    fn mark_notified(&self, id: AlertId) -> impl Future<Output = RepoResult<()>> + Send;
}

pub trait EnergyRepo {
    /// Cree ou remplace le bilan du jour.
    fn upsert(&self, day: &EnergyDay) -> impl Future<Output = RepoResult<()>> + Send;
    fn get(
        &self,
        day: chrono::NaiveDate,
    ) -> impl Future<Output = RepoResult<Option<EnergyDay>>> + Send;
    /// Les `days` derniers jours enregistres, du plus ancien au plus recent.
    fn recent(&self, days: u32) -> impl Future<Output = RepoResult<Vec<EnergyDay>>> + Send;
}

// ------------------------------------------------------------------ R5 : relais Android et messagerie

pub trait AndroidDeviceRepo {
    /// Appareils d'un utilisateur (`None` : tous, pour un administrateur), du plus recent au plus ancien.
    fn list_for_owner(
        &self,
        owner: Option<UserId>,
    ) -> impl Future<Output = RepoResult<Vec<AndroidDevice>>> + Send;
    fn get(
        &self,
        id: AndroidDeviceId,
    ) -> impl Future<Output = RepoResult<Option<AndroidDevice>>> + Send;
    /// Cree l'appareil et renvoie son jeton **en clair, une seule fois** : seule son empreinte SHA-256 est stockee.
    fn create(
        &self,
        owner: UserId,
        d: &AndroidDeviceDraft,
    ) -> impl Future<Output = RepoResult<(AndroidDevice, String)>> + Send;
    /// Nouveau jeton (l'ancien cesse de fonctionner) ; `NotFound` si l'appareil n'existe pas.
    fn regenerate_token(
        &self,
        id: AndroidDeviceId,
    ) -> impl Future<Output = RepoResult<String>> + Send;
    fn delete(&self, id: AndroidDeviceId) -> impl Future<Output = RepoResult<()>> + Send;
    /// Appareil dont le jeton (forme canonique) a cette empreinte.
    fn find_by_token(
        &self,
        token: &str,
    ) -> impl Future<Output = RepoResult<Option<AndroidDevice>>> + Send;
    /// Passe l'appareil en ligne ou hors ligne ; `last_seen = at` dans les deux cas.
    fn set_online(
        &self,
        id: AndroidDeviceId,
        online: bool,
        at: DateTime<Utc>,
    ) -> impl Future<Output = RepoResult<()>> + Send;
    /// L'appareil en ligne vu en dernier (de n'importe quel proprietaire) : celui qui envoie les SMS de la messagerie.
    fn most_recent_online(&self) -> impl Future<Output = RepoResult<Option<AndroidDevice>>> + Send;
    /// Au demarrage du serveur : plus aucune connexion n'existe, tous les appareils passent hors ligne.
    fn set_all_offline(&self) -> impl Future<Output = RepoResult<u64>> + Send;
}

pub trait SmsRepo {
    fn insert(&self, s: &NewSms) -> impl Future<Output = RepoResult<SmsMessage>> + Send;
    /// Les plus recents d'abord ; `device` filtre, sinon tous les appareils de `owner` (`None` : tous).
    fn list(
        &self,
        owner: Option<UserId>,
        device: Option<AndroidDeviceId>,
        limit: i64,
    ) -> impl Future<Output = RepoResult<Vec<SmsMessage>>> + Send;
    fn get(&self, id: SmsId) -> impl Future<Output = RepoResult<Option<SmsMessage>>> + Send;
    /// Marque lus les SMS recus d'un appareil.
    fn mark_read(&self, device: AndroidDeviceId) -> impl Future<Output = RepoResult<u64>> + Send;
    fn unread_count(&self, owner: Option<UserId>) -> impl Future<Output = RepoResult<i64>> + Send;
}

pub trait CallRepo {
    /// Applique un plan du cycle de vie ; renvoie l'entree creee ou modifiee.
    fn apply(&self, plan: &CallPlan) -> impl Future<Output = RepoResult<Option<CallLog>>> + Send;
    /// Appel encore ouvert (sans fin) du meme numero, le plus recent.
    fn find_open(
        &self,
        device: AndroidDeviceId,
        number: &str,
    ) -> impl Future<Output = RepoResult<Option<CallLog>>> + Send;
    fn list(
        &self,
        owner: Option<UserId>,
        device: Option<AndroidDeviceId>,
        limit: i64,
    ) -> impl Future<Output = RepoResult<Vec<CallLog>>> + Send;
    fn get(&self, id: CallId) -> impl Future<Output = RepoResult<Option<CallLog>>> + Send;
}

pub trait SmsOutboxRepo {
    fn enqueue(&self, n: &NewOutbox) -> impl Future<Output = RepoResult<SmsOutbox>> + Send;
    fn get(&self, id: OutboxId) -> impl Future<Output = RepoResult<Option<SmsOutbox>>> + Send;
    /// Etat d'envoi des SMS du navigateur donnes (ceux qui n'ont pas de ligne n'apparaissent pas).
    fn for_sms(&self, ids: &[SmsId]) -> impl Future<Output = RepoResult<Vec<SmsOutbox>>> + Send;
    /// En attente pour cet appareil, du plus ancien au plus recent.
    fn pending_for_device(
        &self,
        device: AndroidDeviceId,
    ) -> impl Future<Output = RepoResult<Vec<SmsOutbox>>> + Send;
    /// Accuse de l'application. Ne change que les lignes `pending` de **cet appareil** : renvoie `None` si l'identifiant est inconnu,
    /// appartient a un autre appareil ou a deja recu un accuse.
    fn ack(
        &self,
        device: AndroidDeviceId,
        id: OutboxId,
        status: OutboxStatus,
        error: &str,
        at: DateTime<Utc>,
    ) -> impl Future<Output = RepoResult<Option<SmsOutbox>>> + Send;
    /// Passe en `failed` (message `error`) les lignes `pending` creees avant `before` ; renvoie les lignes modifiees.
    fn expire(
        &self,
        before: DateTime<Utc>,
        error: &str,
        at: DateTime<Utc>,
    ) -> impl Future<Output = RepoResult<Vec<SmsOutbox>>> + Send;
}

pub trait MessageRepo {
    /// Message `sending` et un destinataire par entree de `deliveries` (`(utilisateur, e-mail, SMS)`), dans une transaction.
    fn create(
        &self,
        sender: UserId,
        subject: &str,
        body: &str,
        channel: crate::messaging::Channel,
        deliveries: &[(UserId, DeliveryStatus, DeliveryStatus)],
    ) -> impl Future<Output = RepoResult<(Message, Vec<Delivery>)>> + Send;
    fn get(&self, id: MessageId) -> impl Future<Output = RepoResult<Option<Message>>> + Send;
    /// Les plus recents d'abord.
    fn list(
        &self,
        limit: i64,
        offset: i64,
    ) -> impl Future<Output = RepoResult<Vec<Message>>> + Send;
    fn deliveries(&self, id: MessageId) -> impl Future<Output = RepoResult<Vec<Delivery>>> + Send;
    /// Messages recus par un compte avec son propre envoi, les plus recents d'abord.
    fn inbox(
        &self,
        user: UserId,
        limit: i64,
    ) -> impl Future<Output = RepoResult<Vec<(Message, Delivery)>>> + Send;
    fn delivery(&self, id: DeliveryId)
    -> impl Future<Output = RepoResult<Option<Delivery>>> + Send;
    /// Met a jour l'etat e-mail et/ou SMS d'un destinataire (`None` : inchange) ; `error` s'ajoute s'il n'est pas vide.
    fn set_delivery(
        &self,
        id: DeliveryId,
        email: Option<DeliveryStatus>,
        sms: Option<DeliveryStatus>,
        error: &str,
    ) -> impl Future<Output = RepoResult<Option<Delivery>>> + Send;
    /// Passe le message en `sent` (avec `sent_at`) s'il n'a plus d'envoi en attente ; renvoie le message dans son etat final.
    fn finish_if_complete(
        &self,
        id: MessageId,
        at: DateTime<Utc>,
    ) -> impl Future<Output = RepoResult<Option<Message>>> + Send;
    fn set_status(
        &self,
        id: MessageId,
        status: MessageStatus,
    ) -> impl Future<Output = RepoResult<()>> + Send;
}

/// Cameras (R6, ADR 0043). Les secrets du brouillon sont deja chiffres.
pub trait CameraRepo {
    /// Par priorite puis par nom.
    fn list(&self) -> impl Future<Output = RepoResult<Vec<Camera>>> + Send;
    fn get(&self, id: CameraId) -> impl Future<Output = RepoResult<Option<Camera>>> + Send;
    fn get_by_uuid(
        &self,
        uuid: CameraUuid,
    ) -> impl Future<Output = RepoResult<Option<Camera>>> + Send;
    /// `uuid` : l'identifiant public voulu (le serveur le tire avant d'enregistrer, car il lie les secrets chiffres) ; `None` : la
    /// base en tire un. `Conflict` s'il existe deja.
    fn create(
        &self,
        uuid: Option<CameraUuid>,
        draft: &CameraDraft,
    ) -> impl Future<Output = RepoResult<Camera>> + Send;
    fn update(
        &self,
        id: CameraId,
        draft: &CameraDraft,
    ) -> impl Future<Output = RepoResult<Camera>> + Send;
    /// Supprime la camera et, en cascade, ses presets, enregistrements, snapshots et evenements.
    fn delete(&self, id: CameraId) -> impl Future<Output = RepoResult<()>> + Send;
    fn touch_last_seen(
        &self,
        id: CameraId,
        at: DateTime<Utc>,
    ) -> impl Future<Output = RepoResult<()>> + Send;
    /// Enregistre ce que la synchronisation ONVIF a appris (fabricant, modele, profil principal, PTZ).
    fn apply_onvif(
        &self,
        id: CameraId,
        info: &OnvifInfo,
    ) -> impl Future<Output = RepoResult<()>> + Send;
    fn set_recording_mode(
        &self,
        id: CameraId,
        mode: RecordingMode,
    ) -> impl Future<Output = RepoResult<()>> + Send;
    fn set_yolo_enabled(
        &self,
        id: CameraId,
        enabled: bool,
    ) -> impl Future<Output = RepoResult<()>> + Send;
}

pub trait PtzPresetRepo {
    /// Par position puis par nom.
    fn list(&self, camera: CameraId) -> impl Future<Output = RepoResult<Vec<PtzPreset>>> + Send;
    /// `Conflict` si le nom existe deja pour cette camera.
    fn create(
        &self,
        camera: CameraId,
        draft: &PtzPresetDraft,
    ) -> impl Future<Output = RepoResult<PtzPreset>> + Send;
    /// Cree le preset ou, s'il existe deja un preset de ce nom pour cette camera, remplace son jeton et sa position (comme la reference).
    fn upsert(
        &self,
        camera: CameraId,
        draft: &PtzPresetDraft,
    ) -> impl Future<Output = RepoResult<PtzPreset>> + Send;
    fn get(&self, id: PtzPresetId) -> impl Future<Output = RepoResult<Option<PtzPreset>>> + Send;
    fn delete(&self, id: PtzPresetId) -> impl Future<Output = RepoResult<()>> + Send;
}

pub trait RecordingRepo {
    /// `Conflict` si le fichier est deja indexe.
    fn create(
        &self,
        camera: CameraId,
        new: &NewRecording,
    ) -> impl Future<Output = RepoResult<Recording>> + Send;
    fn get(&self, id: RecordingId) -> impl Future<Output = RepoResult<Option<Recording>>> + Send;
    /// Les plus recents d'abord.
    fn list(
        &self,
        camera: CameraId,
        limit: i64,
    ) -> impl Future<Output = RepoResult<Vec<Recording>>> + Send;
    /// Les segments dont le debut tombe dans `[from, to)`, du plus ancien au plus recent (page d'une journee).
    fn list_between(
        &self,
        camera: CameraId,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
    ) -> impl Future<Output = RepoResult<Vec<Recording>>> + Send;
    /// Segments pas encore archives (`pending`, `uploading`, `error`), les plus anciens d'abord.
    fn list_unarchived(
        &self,
        limit: i64,
    ) -> impl Future<Output = RepoResult<Vec<Recording>>> + Send;
    fn path_known(&self, path: &str) -> impl Future<Output = RepoResult<bool>> + Send;
    fn mark_uploading(&self, id: RecordingId) -> impl Future<Output = RepoResult<()>> + Send;
    fn mark_done(
        &self,
        id: RecordingId,
        reduct_ts: i64,
    ) -> impl Future<Output = RepoResult<()>> + Send;
    fn mark_error(
        &self,
        id: RecordingId,
        message: &str,
    ) -> impl Future<Output = RepoResult<()>> + Send;
    fn delete(&self, id: RecordingId) -> impl Future<Output = RepoResult<()>> + Send;
}

pub trait SnapshotRepo {
    fn create(
        &self,
        camera: CameraId,
        taken_at: DateTime<Utc>,
        file: &str,
        size_bytes: i32,
    ) -> impl Future<Output = RepoResult<Snapshot>> + Send;
    fn get(&self, id: SnapshotId) -> impl Future<Output = RepoResult<Option<Snapshot>>> + Send;
    /// Les plus recents d'abord.
    fn list(
        &self,
        camera: CameraId,
        limit: i64,
    ) -> impl Future<Output = RepoResult<Vec<Snapshot>>> + Send;
    /// Snapshots de toutes les cameras (ou d'une seule), entre `from` (inclus) et `to` (exclu) si donnes, les plus recents d'abord.
    fn list_filtered(
        &self,
        camera: Option<CameraId>,
        from: Option<DateTime<Utc>>,
        to: Option<DateTime<Utc>>,
        limit: i64,
    ) -> impl Future<Output = RepoResult<Vec<Snapshot>>> + Send;
    /// Les snapshots d'une camera au-dela des `keep` plus recents (a supprimer), les plus anciens d'abord.
    fn excess(
        &self,
        camera: CameraId,
        keep: i64,
    ) -> impl Future<Output = RepoResult<Vec<Snapshot>>> + Send;
    fn delete(&self, id: SnapshotId) -> impl Future<Output = RepoResult<()>> + Send;
}

/// Mouvements et detections d'objets.
pub trait EventRepo {
    fn record_motion(
        &self,
        camera: CameraId,
        at: DateTime<Utc>,
        confidence: f64,
    ) -> impl Future<Output = RepoResult<MotionEvent>> + Send;
    fn list_motion(
        &self,
        camera: CameraId,
        limit: i64,
    ) -> impl Future<Output = RepoResult<Vec<MotionEvent>>> + Send;
    /// Un mouvement a-t-il eu lieu entre `from` et `to` (type d'un segment enregistre) ?
    fn motion_between(
        &self,
        camera: CameraId,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
    ) -> impl Future<Output = RepoResult<bool>> + Send;
    fn record_detection(
        &self,
        camera: CameraId,
        at: DateTime<Utc>,
        trigger: DetectionTrigger,
        labels: &[Detection],
        frame_path: &str,
    ) -> impl Future<Output = RepoResult<DetectionEvent>> + Send;
    fn list_detections(
        &self,
        camera: CameraId,
        limit: i64,
    ) -> impl Future<Output = RepoResult<Vec<DetectionEvent>>> + Send;
    fn get_detection(
        &self,
        id: DetectionEventId,
    ) -> impl Future<Output = RepoResult<Option<DetectionEvent>>> + Send;
}

pub trait DetectorConfigRepo {
    fn get(&self) -> impl Future<Output = RepoResult<DetectorConfig>> + Send;
    fn update(
        &self,
        config: &DetectorConfig,
    ) -> impl Future<Output = RepoResult<DetectorConfig>> + Send;
}

/// Jetons d'acces MCP (ADR 0059). La valeur d'un jeton n'arrive jamais ici : seulement son empreinte SHA-256.
pub trait McpTokenRepo {
    /// Enregistre un jeton (`hash` : 32 octets, `prefix` : le debut visible de la valeur).
    fn create(
        &self,
        new: &NewMcpToken,
        hash: &[u8],
        prefix: &str,
        now: DateTime<Utc>,
        created_by: Option<UserId>,
    ) -> impl Future<Output = RepoResult<McpToken>> + Send;
    /// Tous les jetons, les plus recents d'abord (actifs, expires et revoques).
    fn list(&self) -> impl Future<Output = RepoResult<Vec<McpToken>>> + Send;
    /// Le jeton de cette empreinte s'il est **actif** a `now` (ni revoque, ni expire).
    fn find_active(
        &self,
        hash: &[u8],
        now: DateTime<Utc>,
    ) -> impl Future<Output = RepoResult<Option<McpToken>>> + Send;
    /// Note l'usage ; n'ecrit pas plus d'une fois par minute et par jeton.
    fn touch(
        &self,
        id: McpTokenId,
        now: DateTime<Utc>,
    ) -> impl Future<Output = RepoResult<()>> + Send;
    /// Revoque ; `true` si le jeton etait actif (une revocation deja faite ne change pas sa date).
    fn revoke(
        &self,
        id: McpTokenId,
        now: DateTime<Utc>,
    ) -> impl Future<Output = RepoResult<bool>> + Send;
}

/// Journal d'audit MCP.
pub trait McpAuditRepo {
    /// Ecrit une ligne et renvoie son numero. Une action qui change quelque chose ecrit sa ligne **avant** de s'executer, avec
    /// l'issue `pending`, puis la ferme par [`McpAuditRepo::complete`].
    fn record(
        &self,
        entry: &NewAuditEntry,
        now: DateTime<Utc>,
    ) -> impl Future<Output = RepoResult<i64>> + Send;
    /// Fixe l'issue et le detail d'une ligne ecrite `pending`.
    fn complete(
        &self,
        id: i64,
        outcome: &str,
        detail: &str,
    ) -> impl Future<Output = RepoResult<()>> + Send;
    /// Les plus recentes d'abord, d'un seul type si `kind` est donne ; `limit` est borne par l'appelant.
    fn recent(
        &self,
        kind: Option<AuditKind>,
        limit: i64,
    ) -> impl Future<Output = RepoResult<Vec<McpAuditEntry>>> + Send;
    /// Supprime ce qui est plus ancien que `before` ; renvoie le nombre de lignes.
    fn purge_before(&self, before: DateTime<Utc>) -> impl Future<Output = RepoResult<u64>> + Send;
}
