// SPDX-License-Identifier: AGPL-3.0-or-later
//! Storage ports: the adapters (`dominet-store`) implement them, the server consumes them.
//!
//! The methods return `impl Future + Send` so as to stay usable from Tokio tasks.

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
use crate::stream_access::{
    ActiveStreamAccess, NewStreamCredential, StreamAccess, StreamAccessId, StreamAccessKind,
};
use crate::user::{Credentials, NewUser, User, UserId, UserProfile};

pub type RepoResult<T> = Result<T, RepoError>;

pub trait UserRepo {
    /// Creates an account. `Conflict` if the name already exists (case ignored).
    fn create(&self, new: &NewUser) -> impl Future<Output = RepoResult<User>> + Send;
    fn find_by_id(&self, id: UserId) -> impl Future<Output = RepoResult<Option<User>>> + Send;
    /// Case-insensitive search.
    fn find_by_username(
        &self,
        username: &str,
    ) -> impl Future<Output = RepoResult<Option<User>>> + Send;
    /// Account + password hash (login).
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
    /// Replaces the hash and dates the change (invalidates the old sessions on the server side).
    fn set_password_hash(
        &self,
        id: UserId,
        hash: &str,
    ) -> impl Future<Output = RepoResult<()>> + Send;
    fn touch_last_login(&self, id: UserId) -> impl Future<Output = RepoResult<()>> + Send;
    fn delete(&self, id: UserId) -> impl Future<Output = RepoResult<()>> + Send;
    /// Number of active administrator accounts (to forbid removing the last one).
    fn count_active_admins(&self) -> impl Future<Output = RepoResult<i64>> + Send;
}

pub trait SettingRepo {
    /// All the settings, sorted by category then key.
    fn list(&self) -> impl Future<Output = RepoResult<Vec<AppSetting>>> + Send;
    fn get(&self, key: &str) -> impl Future<Output = RepoResult<Option<AppSetting>>> + Send;
    /// Creates the setting or replaces its value, type, category and description.
    /// `stored_value` is the value already ready to store (encrypted if `draft.is_secret`).
    fn upsert(
        &self,
        draft: &SettingDraft,
        stored_value: &str,
    ) -> impl Future<Output = RepoResult<AppSetting>> + Send;
    /// Creates the setting only if it does not exist; returns `true` if created (initial data).
    fn insert_if_absent(
        &self,
        draft: &SettingDraft,
        stored_value: &str,
    ) -> impl Future<Output = RepoResult<bool>> + Send;
    fn delete(&self, key: &str) -> impl Future<Output = RepoResult<()>> + Send;
}

pub trait NavbarRepo {
    /// Entries sorted: top then bottom, by order then label.
    fn list(&self) -> impl Future<Output = RepoResult<Vec<NavbarItem>>> + Send;
    fn get(&self, id: NavbarItemId) -> impl Future<Output = RepoResult<Option<NavbarItem>>> + Send;
    fn create(&self, draft: &NavbarDraft) -> impl Future<Output = RepoResult<NavbarItem>> + Send;
    fn update(
        &self,
        id: NavbarItemId,
        draft: &NavbarDraft,
    ) -> impl Future<Output = RepoResult<NavbarItem>> + Send;
    fn delete(&self, id: NavbarItemId) -> impl Future<Output = RepoResult<()>> + Send;
    /// Creates the entry if none already has the same position and label; `true` if created.
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

/// Prometheus alert rules (R7 lot 2). `list` returns the file's order: group then name.
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
    /// Creates the rule if none has this name; `true` if created (seed of the shipped rules).
    fn insert_if_absent(
        &self,
        draft: &PrometheusRuleDraft,
    ) -> impl Future<Output = RepoResult<bool>> + Send;
}

/// Grafana alert rules (R7 lot 2). `list` returns the file's order: folder, group then title.
pub trait GrafanaRuleRepo {
    fn list(&self) -> impl Future<Output = RepoResult<Vec<GrafanaRule>>> + Send;
    fn get(
        &self,
        id: GrafanaRuleId,
    ) -> impl Future<Output = RepoResult<Option<GrafanaRule>>> + Send;
    /// Creates the rule; an empty `uid` is derived from the title and made unique (`title`, `title-1`, ...).
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
    /// Creates the rule if none has this `uid`; `true` if created.
    fn insert_if_absent(
        &self,
        draft: &GrafanaRuleDraft,
    ) -> impl Future<Output = RepoResult<bool>> + Send;
}

/// Log of login attempts (anti brute-force lock-out).
/// Dashboards. An account's first dashboard is always its default dashboard; designating a dashboard as
/// default removes that status from the others of the same account, in the same transaction.
pub trait DashboardRepo {
    /// An account's dashboards, by name.
    fn list_for_owner(
        &self,
        owner: UserId,
    ) -> impl Future<Output = RepoResult<Vec<Dashboard>>> + Send;
    /// All the dashboards (administration), by owner then by name.
    fn list_all(&self) -> impl Future<Output = RepoResult<Vec<Dashboard>>> + Send;
    fn get(&self, id: DashboardId) -> impl Future<Output = RepoResult<Option<Dashboard>>> + Send;
    /// `NotFound` if the owner does not exist.
    fn create(
        &self,
        owner: UserId,
        draft: &DashboardDraft,
    ) -> impl Future<Output = RepoResult<Dashboard>> + Send;
    /// Renames; `draft.is_default` can only enable the status (a default dashboard stays so until another one becomes it).
    fn update(
        &self,
        id: DashboardId,
        draft: &DashboardDraft,
    ) -> impl Future<Output = RepoResult<Dashboard>> + Send;
    /// Deletes the dashboard and its widgets.
    fn delete(&self, id: DashboardId) -> impl Future<Output = RepoResult<()>> + Send;
}

/// Widgets of a dashboard. `Conflict { constraint: "widget_no_overlap" }` if the rectangle overlaps another widget of the dashboard.
pub trait WidgetRepo {
    /// Widgets of a dashboard, by row then column.
    fn list(&self, dashboard: DashboardId) -> impl Future<Output = RepoResult<Vec<Widget>>> + Send;
    fn get(&self, id: WidgetId) -> impl Future<Output = RepoResult<Option<Widget>>> + Send;
    /// `NotFound` if the dashboard does not exist; `Conflict` if the rectangle overlaps another widget. The widget writes
    /// of one dashboard are serialised (dashboard lock).
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
    /// Moves the widget (position only, size unchanged).
    fn move_to(
        &self,
        id: WidgetId,
        col: i16,
        row: i16,
    ) -> impl Future<Output = RepoResult<Widget>> + Send;
    fn delete(&self, id: WidgetId) -> impl Future<Output = RepoResult<()>> + Send;
}

pub trait LoginAttemptRepo {
    /// Records an attempt; the name is stored in lower case.
    fn record(
        &self,
        username: &str,
        ip: &str,
        succeeded: bool,
    ) -> impl Future<Output = RepoResult<()>> + Send;
    /// Failures since `since` (see `AttemptStats` for the three counters).
    fn stats(
        &self,
        username: &str,
        ip: &str,
        since: DateTime<Utc>,
    ) -> impl Future<Output = RepoResult<AttemptStats>> + Send;
    /// Deletes the attempts older than `before`; returns the number of rows deleted.
    fn purge_before(&self, before: DateTime<Utc>) -> impl Future<Output = RepoResult<u64>> + Send;
}

/// Criteria of the devices list.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DeviceFilter {
    pub group: Option<DeviceGroupId>,
    pub location: Option<LocationId>,
    pub device_type: Option<DeviceType>,
    /// Keep only the active devices.
    pub active_only: bool,
}

/// Devices. The operations `apply_state`, `set_available`, `upsert_discovered`, `upsert_auto` and `touch_by_topic` are those of
/// MQTT listening: they take over the reference implementation's semantics (see `device.rs`) and are **idempotent**
/// (QoS 1 duplicates change nothing but `last_seen`).
pub trait DeviceRepo {
    /// Devices by name.
    fn list(&self, filter: &DeviceFilter) -> impl Future<Output = RepoResult<Vec<Device>>> + Send;
    fn get(&self, id: DeviceId) -> impl Future<Output = RepoResult<Option<Device>>> + Send;
    fn find_by_name(
        &self,
        friendly_name: &str,
    ) -> impl Future<Output = RepoResult<Option<Device>>> + Send;
    /// `Conflict` if the name already exists.
    fn create(&self, draft: &DeviceDraft) -> impl Future<Output = RepoResult<Device>> + Send;
    /// Modifies the parameters; the state, the availability and the discovery do not change.
    fn update(
        &self,
        id: DeviceId,
        draft: &DeviceDraft,
    ) -> impl Future<Output = RepoResult<Device>> + Send;
    fn delete(&self, id: DeviceId) -> impl Future<Output = RepoResult<()>> + Send;
    /// `bridge/devices`: creates the device (type deduced, `base_topic` prefix, masked key `update`) or updates its description.
    /// Returns the device and `true` if it has just been created.
    fn upsert_discovered(
        &self,
        d: &Discovered,
        base_topic: &str,
    ) -> impl Future<Output = RepoResult<(Device, bool)>> + Send;
    /// State message: **replaces** the state, `last_seen = now`, `available = true`; only if the device is active.
    /// `None` if there is no active device of that name.
    fn apply_state(
        &self,
        friendly_name: &str,
        state: &serde_json::Value,
    ) -> impl Future<Output = RepoResult<Option<Device>>> + Send;
    /// Availability message (no effect on `last_seen`); `None` if the name is unknown.
    fn set_available(
        &self,
        friendly_name: &str,
        available: bool,
    ) -> impl Future<Output = RepoResult<Option<Device>>> + Send;
    /// MQTT source as `auto_device`: creates or updates (and **reactivates**) the device, replaces its state.
    fn upsert_auto(
        &self,
        friendly_name: &str,
        base_topic: &str,
        device_type: DeviceType,
        state: &serde_json::Value,
    ) -> impl Future<Output = RepoResult<(Device, bool)>> + Send;
    /// Marks as seen (available, `last_seen`) the active devices whose `{base_topic}/{name}` or `base_topic` equals the topic.
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
    /// The group's devices stay, without a group.
    fn delete(&self, id: DeviceGroupId) -> impl Future<Output = RepoResult<()>> + Send;
}

pub trait DeviceLabelRepo {
    /// By key.
    fn list(&self) -> impl Future<Output = RepoResult<Vec<DeviceLabel>>> + Send;
    fn get(
        &self,
        id: DeviceLabelId,
    ) -> impl Future<Output = RepoResult<Option<DeviceLabel>>> + Send;
    /// `Conflict` if the key already exists.
    fn create(&self, d: &DeviceLabelDraft) -> impl Future<Output = RepoResult<DeviceLabel>> + Send;
    fn update(
        &self,
        id: DeviceLabelId,
        d: &DeviceLabelDraft,
    ) -> impl Future<Output = RepoResult<DeviceLabel>> + Send;
    fn delete(&self, id: DeviceLabelId) -> impl Future<Output = RepoResult<()>> + Send;
    /// Creates the labels whose key does not exist yet; returns the number created.
    fn create_missing(
        &self,
        drafts: &[DeviceLabelDraft],
    ) -> impl Future<Output = RepoResult<u64>> + Send;
    /// All the keys present in the state of at least one device (for `seed-labels`).
    fn state_keys(&self) -> impl Future<Output = RepoResult<Vec<String>>> + Send;
}

/// History of the tracked keys.
pub trait StateLogRepo {
    /// Writes the rows at instant `ts` (to the second); an existing row of the same (device, key, instant) is **replaced**.
    /// Returns the number of rows written.
    fn record(
        &self,
        device: DeviceId,
        ts: DateTime<Utc>,
        entries: &[LogEntry],
    ) -> impl Future<Output = RepoResult<u64>> + Send;
    /// Points of a key since `since`, from oldest to most recent, at most `limit`.
    fn points(
        &self,
        device: DeviceId,
        key: &str,
        since: DateTime<Utc>,
        limit: i64,
    ) -> impl Future<Output = RepoResult<Vec<LogPoint>>> + Send;
    /// Smallest and largest **numeric** value of a key over `[from, to[` (text values are ignored); `None` without
    /// any value. This is what the energy summary aggregates.
    fn min_max(
        &self,
        device: DeviceId,
        key: &str,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
    ) -> impl Future<Output = RepoResult<(Option<f64>, Option<f64>)>> + Send;
    /// Deletes the rows older than `before`; returns the number deleted.
    fn purge_before(&self, before: DateTime<Utc>) -> impl Future<Output = RepoResult<u64>> + Send;
}

pub trait DeviceImageRepo {
    /// `NotFound` if the device does not exist.
    fn put(
        &self,
        device: DeviceId,
        content_type: &str,
        data: &[u8],
    ) -> impl Future<Output = RepoResult<()>> + Send;
    fn get(&self, device: DeviceId)
    -> impl Future<Output = RepoResult<Option<DeviceImage>>> + Send;
    fn delete(&self, device: DeviceId) -> impl Future<Output = RepoResult<()>> + Send;
    /// Devices that have an image, with its update date (without reading the images).
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

// ------------------------------------------------------------------ R4: scenes, automations, alerts, energy

pub trait SceneRepo {
    /// By name, with their actions (in publication order).
    fn list(&self) -> impl Future<Output = RepoResult<Vec<Scene>>> + Send;
    fn get(&self, id: SceneId) -> impl Future<Output = RepoResult<Option<Scene>>> + Send;
    fn find_by_name(&self, name: &str) -> impl Future<Output = RepoResult<Option<Scene>>> + Send;
    /// `Conflict` if the name already exists or if a device does not exist.
    fn create(&self, d: &SceneDraft) -> impl Future<Output = RepoResult<Scene>> + Send;
    /// Also replaces the actions, in a single transaction.
    fn update(&self, id: SceneId, d: &SceneDraft)
    -> impl Future<Output = RepoResult<Scene>> + Send;
    /// The widgets that targeted it lose their scene (`ON DELETE SET NULL`).
    fn delete(&self, id: SceneId) -> impl Future<Output = RepoResult<()>> + Send;
}

pub trait AutomationRepo {
    /// By name.
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
    /// One more trigger: `trigger_count + 1` and `last_triggered = at`.
    fn record_trigger(
        &self,
        id: AutomationId,
        at: DateTime<Utc>,
    ) -> impl Future<Output = RepoResult<()>> + Send;
}

/// Execution log of the automations.
pub trait AutomationRunRepo {
    fn record(&self, run: &AutomationRun) -> impl Future<Output = RepoResult<()>> + Send;
    /// Most recent first.
    fn recent(
        &self,
        automation: AutomationId,
        limit: i64,
    ) -> impl Future<Output = RepoResult<Vec<AutomationRun>>> + Send;
    /// All automations together, most recent first.
    fn recent_all(&self, limit: i64)
    -> impl Future<Output = RepoResult<Vec<AutomationRun>>> + Send;
    fn purge_before(&self, before: DateTime<Utc>) -> impl Future<Output = RepoResult<u64>> + Send;
}

/// Scheduler jobs.
pub trait ScheduleRepo {
    /// All, by name.
    fn list(&self) -> impl Future<Output = RepoResult<Vec<Job>>> + Send;
    /// Creates the job of this name or replaces it. If the schedule, the task, its arguments, the time zone or the activation change,
    /// the next run is forgotten (to be recomputed); otherwise it is kept.
    fn upsert(&self, d: &JobDraft) -> impl Future<Output = RepoResult<Job>> + Send;
    /// Deletes the **automation** jobs that are not in `keep`; returns their number.
    fn prune_automation_jobs(
        &self,
        keep: &[AutomationId],
    ) -> impl Future<Output = RepoResult<u64>> + Send;
    /// Stores the next run (`None` = to recompute).
    fn set_next(
        &self,
        id: JobId,
        next: Option<DateTime<Utc>>,
    ) -> impl Future<Output = RepoResult<()>> + Send;
    /// End of a run: `ran_at` if the task ran (not for a missed run), its status, the next run.
    fn record(
        &self,
        id: JobId,
        ran_at: Option<DateTime<Utc>>,
        status: &str,
        next: Option<DateTime<Utc>>,
    ) -> impl Future<Output = RepoResult<()>> + Send;
    /// Next run of the active automation jobs.
    fn automation_next_runs(
        &self,
    ) -> impl Future<Output = RepoResult<Vec<(AutomationId, Option<DateTime<Utc>>)>>> + Send;
}

pub trait AlertRuleRepo {
    /// By device then by name.
    fn list(&self) -> impl Future<Output = RepoResult<Vec<AlertRule>>> + Send;
    /// The **enabled** rules of a device (evaluated at every state).
    fn list_enabled_for_device(
        &self,
        device: DeviceId,
    ) -> impl Future<Output = RepoResult<Vec<AlertRule>>> + Send;
    fn get(&self, id: AlertRuleId) -> impl Future<Output = RepoResult<Option<AlertRule>>> + Send;
    /// `Conflict` if the device does not exist.
    fn create(&self, d: &AlertRuleDraft) -> impl Future<Output = RepoResult<AlertRule>> + Send;
    fn update(
        &self,
        id: AlertRuleId,
        d: &AlertRuleDraft,
    ) -> impl Future<Output = RepoResult<AlertRule>> + Send;
    /// The alerts already created stay, without a rule.
    fn delete(&self, id: AlertRuleId) -> impl Future<Output = RepoResult<()>> + Send;
}

/// An alert to create.
#[derive(Debug, Clone, PartialEq)]
pub struct NewAlert {
    pub rule: AlertRuleId,
    pub device: DeviceId,
    pub severity: Severity,
    pub value_triggered: String,
    pub fired_at: DateTime<Utc>,
}

pub trait AlertRepo {
    /// Most recent first; `None` = all statuses.
    fn list(
        &self,
        status: Option<AlertStatus>,
        limit: i64,
    ) -> impl Future<Output = RepoResult<Vec<Alert>>> + Send;
    /// The most recent **ongoing** alert of the rule for this device (the acknowledged and resolved ones are not there).
    fn latest_firing(
        &self,
        rule: AlertRuleId,
        device: DeviceId,
    ) -> impl Future<Output = RepoResult<Option<Alert>>> + Send;
    fn create(&self, new: &NewAlert) -> impl Future<Output = RepoResult<Alert>> + Send;
    /// Resolves the ongoing alerts of the rule for this device; returns their number.
    fn resolve_firing(
        &self,
        rule: AlertRuleId,
        device: DeviceId,
        at: DateTime<Utc>,
    ) -> impl Future<Output = RepoResult<u64>> + Send;
    /// Acknowledges an **ongoing** alert; `None` if it does not exist or is no longer ongoing.
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
    /// Creates or replaces the day's summary.
    fn upsert(&self, day: &EnergyDay) -> impl Future<Output = RepoResult<()>> + Send;
    fn get(
        &self,
        day: chrono::NaiveDate,
    ) -> impl Future<Output = RepoResult<Option<EnergyDay>>> + Send;
    /// The last `days` recorded days, from oldest to most recent.
    fn recent(&self, days: u32) -> impl Future<Output = RepoResult<Vec<EnergyDay>>> + Send;
}

// ------------------------------------------------------------------ R5: Android relay and messaging

pub trait AndroidDeviceRepo {
    /// A user's devices (`None`: all, for an administrator), most recent to oldest.
    fn list_for_owner(
        &self,
        owner: Option<UserId>,
    ) -> impl Future<Output = RepoResult<Vec<AndroidDevice>>> + Send;
    fn get(
        &self,
        id: AndroidDeviceId,
    ) -> impl Future<Output = RepoResult<Option<AndroidDevice>>> + Send;
    /// Creates the device and returns its token **in clear, once**: only its SHA-256 fingerprint is stored.
    fn create(
        &self,
        owner: UserId,
        d: &AndroidDeviceDraft,
    ) -> impl Future<Output = RepoResult<(AndroidDevice, String)>> + Send;
    /// New token (the old one stops working); `NotFound` if the device does not exist.
    fn regenerate_token(
        &self,
        id: AndroidDeviceId,
    ) -> impl Future<Output = RepoResult<String>> + Send;
    fn delete(&self, id: AndroidDeviceId) -> impl Future<Output = RepoResult<()>> + Send;
    /// Device whose token (canonical form) has this fingerprint.
    fn find_by_token(
        &self,
        token: &str,
    ) -> impl Future<Output = RepoResult<Option<AndroidDevice>>> + Send;
    /// Sets the device online or offline; `last_seen = at` in both cases.
    fn set_online(
        &self,
        id: AndroidDeviceId,
        online: bool,
        at: DateTime<Utc>,
    ) -> impl Future<Output = RepoResult<()>> + Send;
    /// The online device seen last (of any owner): the one that sends the messaging SMS.
    fn most_recent_online(&self) -> impl Future<Output = RepoResult<Option<AndroidDevice>>> + Send;
    /// At server start-up: no connection exists any more, all the devices go offline.
    fn set_all_offline(&self) -> impl Future<Output = RepoResult<u64>> + Send;
}

pub trait SmsRepo {
    fn insert(&self, s: &NewSms) -> impl Future<Output = RepoResult<SmsMessage>> + Send;
    /// Most recent first; `device` filters, otherwise all the devices of `owner` (`None`: all).
    fn list(
        &self,
        owner: Option<UserId>,
        device: Option<AndroidDeviceId>,
        limit: i64,
    ) -> impl Future<Output = RepoResult<Vec<SmsMessage>>> + Send;
    fn get(&self, id: SmsId) -> impl Future<Output = RepoResult<Option<SmsMessage>>> + Send;
    /// Marks as read the SMS received by a device.
    fn mark_read(&self, device: AndroidDeviceId) -> impl Future<Output = RepoResult<u64>> + Send;
    fn unread_count(&self, owner: Option<UserId>) -> impl Future<Output = RepoResult<i64>> + Send;
}

pub trait CallRepo {
    /// Applies a life-cycle plan; returns the entry created or modified.
    fn apply(&self, plan: &CallPlan) -> impl Future<Output = RepoResult<Option<CallLog>>> + Send;
    /// Call still open (without an end) of the same number, the most recent.
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
    /// Sending state of the given browser SMS (those without a row do not appear).
    fn for_sms(&self, ids: &[SmsId]) -> impl Future<Output = RepoResult<Vec<SmsOutbox>>> + Send;
    /// Pending for this device, from oldest to most recent.
    fn pending_for_device(
        &self,
        device: AndroidDeviceId,
    ) -> impl Future<Output = RepoResult<Vec<SmsOutbox>>> + Send;
    /// Acknowledgement from the application. Only changes the `pending` rows of **this device**: returns `None` if the identifier is unknown,
    /// belongs to another device or has already received an acknowledgement.
    fn ack(
        &self,
        device: AndroidDeviceId,
        id: OutboxId,
        status: OutboxStatus,
        error: &str,
        at: DateTime<Utc>,
    ) -> impl Future<Output = RepoResult<Option<SmsOutbox>>> + Send;
    /// Sets to `failed` (message `error`) the `pending` rows created before `before`; returns the rows modified.
    fn expire(
        &self,
        before: DateTime<Utc>,
        error: &str,
        at: DateTime<Utc>,
    ) -> impl Future<Output = RepoResult<Vec<SmsOutbox>>> + Send;
}

pub trait MessageRepo {
    /// `sending` message and one recipient per entry of `deliveries` (`(user, e-mail, SMS)`), in a transaction.
    fn create(
        &self,
        sender: UserId,
        subject: &str,
        body: &str,
        channel: crate::messaging::Channel,
        deliveries: &[(UserId, DeliveryStatus, DeliveryStatus)],
    ) -> impl Future<Output = RepoResult<(Message, Vec<Delivery>)>> + Send;
    fn get(&self, id: MessageId) -> impl Future<Output = RepoResult<Option<Message>>> + Send;
    /// Most recent first.
    fn list(
        &self,
        limit: i64,
        offset: i64,
    ) -> impl Future<Output = RepoResult<Vec<Message>>> + Send;
    fn deliveries(&self, id: MessageId) -> impl Future<Output = RepoResult<Vec<Delivery>>> + Send;
    /// Messages received by an account with its own sending, most recent first.
    fn inbox(
        &self,
        user: UserId,
        limit: i64,
    ) -> impl Future<Output = RepoResult<Vec<(Message, Delivery)>>> + Send;
    fn delivery(&self, id: DeliveryId)
    -> impl Future<Output = RepoResult<Option<Delivery>>> + Send;
    /// Updates the e-mail and/or SMS state of a recipient (`None`: unchanged); `error` is appended if it is not empty.
    fn set_delivery(
        &self,
        id: DeliveryId,
        email: Option<DeliveryStatus>,
        sms: Option<DeliveryStatus>,
        error: &str,
    ) -> impl Future<Output = RepoResult<Option<Delivery>>> + Send;
    /// Sets the message to `sent` (with `sent_at`) if it has no pending sending any more; returns the message in its final state.
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

/// Cameras (R6, ADR 0043). The draft's secrets are already encrypted.
pub trait CameraRepo {
    /// By priority then by name.
    fn list(&self) -> impl Future<Output = RepoResult<Vec<Camera>>> + Send;
    fn get(&self, id: CameraId) -> impl Future<Output = RepoResult<Option<Camera>>> + Send;
    fn get_by_uuid(
        &self,
        uuid: CameraUuid,
    ) -> impl Future<Output = RepoResult<Option<Camera>>> + Send;
    /// `uuid`: the wanted public identifier (the server draws it before saving, because it binds the encrypted secrets); `None`: the
    /// database draws one. `Conflict` if it already exists.
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
    /// Deletes the camera and, in cascade, its presets, recordings, snapshots and events.
    fn delete(&self, id: CameraId) -> impl Future<Output = RepoResult<()>> + Send;
    fn touch_last_seen(
        &self,
        id: CameraId,
        at: DateTime<Utc>,
    ) -> impl Future<Output = RepoResult<()>> + Send;
    /// Records what the ONVIF synchronisation learned (manufacturer, model, main profile, PTZ).
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
    /// By position then by name.
    fn list(&self, camera: CameraId) -> impl Future<Output = RepoResult<Vec<PtzPreset>>> + Send;
    /// `Conflict` if the name already exists for this camera.
    fn create(
        &self,
        camera: CameraId,
        draft: &PtzPresetDraft,
    ) -> impl Future<Output = RepoResult<PtzPreset>> + Send;
    /// Creates the preset or, if a preset of this name already exists for this camera, replaces its token and position (like the reference).
    fn upsert(
        &self,
        camera: CameraId,
        draft: &PtzPresetDraft,
    ) -> impl Future<Output = RepoResult<PtzPreset>> + Send;
    fn get(&self, id: PtzPresetId) -> impl Future<Output = RepoResult<Option<PtzPreset>>> + Send;
    fn delete(&self, id: PtzPresetId) -> impl Future<Output = RepoResult<()>> + Send;
}

pub trait RecordingRepo {
    /// `Conflict` if the file is already indexed.
    fn create(
        &self,
        camera: CameraId,
        new: &NewRecording,
    ) -> impl Future<Output = RepoResult<Recording>> + Send;
    fn get(&self, id: RecordingId) -> impl Future<Output = RepoResult<Option<Recording>>> + Send;
    /// Most recent first.
    fn list(
        &self,
        camera: CameraId,
        limit: i64,
    ) -> impl Future<Output = RepoResult<Vec<Recording>>> + Send;
    /// The segments whose start falls in `[from, to)`, from oldest to most recent (page of a day).
    fn list_between(
        &self,
        camera: CameraId,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
    ) -> impl Future<Output = RepoResult<Vec<Recording>>> + Send;
    /// Segments not yet archived (`pending`, `uploading`, `error`), oldest first.
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
    /// Most recent first.
    fn list(
        &self,
        camera: CameraId,
        limit: i64,
    ) -> impl Future<Output = RepoResult<Vec<Snapshot>>> + Send;
    /// Snapshots of all the cameras (or of just one), between `from` (inclusive) and `to` (exclusive) if given, most recent first.
    fn list_filtered(
        &self,
        camera: Option<CameraId>,
        from: Option<DateTime<Utc>>,
        to: Option<DateTime<Utc>>,
        limit: i64,
    ) -> impl Future<Output = RepoResult<Vec<Snapshot>>> + Send;
    /// The snapshots of a camera beyond the `keep` most recent ones (to delete), oldest first.
    fn excess(
        &self,
        camera: CameraId,
        keep: i64,
    ) -> impl Future<Output = RepoResult<Vec<Snapshot>>> + Send;
    fn delete(&self, id: SnapshotId) -> impl Future<Output = RepoResult<()>> + Send;
}

/// Motions and object detections.
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
    /// Did a motion take place between `from` and `to` (type of a recorded segment)?
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

/// MCP access tokens (ADR 0059). A token's value never arrives here: only its SHA-256 fingerprint.
pub trait McpTokenRepo {
    /// Records a token (`hash`: 32 bytes, `prefix`: the visible beginning of the value).
    fn create(
        &self,
        new: &NewMcpToken,
        hash: &[u8],
        prefix: &str,
        now: DateTime<Utc>,
        created_by: Option<UserId>,
    ) -> impl Future<Output = RepoResult<McpToken>> + Send;
    /// All the tokens, most recent first (active, expired and revoked).
    fn list(&self) -> impl Future<Output = RepoResult<Vec<McpToken>>> + Send;
    /// The token with this fingerprint if it is **active** at `now` (neither revoked nor expired).
    fn find_active(
        &self,
        hash: &[u8],
        now: DateTime<Utc>,
    ) -> impl Future<Output = RepoResult<Option<McpToken>>> + Send;
    /// Notes the usage; writes at most once a minute per token.
    fn touch(
        &self,
        id: McpTokenId,
        now: DateTime<Utc>,
    ) -> impl Future<Output = RepoResult<()>> + Send;
    /// Revokes; `true` if the token was active (a revocation already done does not change its date).
    fn revoke(
        &self,
        id: McpTokenId,
        now: DateTime<Utc>,
    ) -> impl Future<Output = RepoResult<bool>> + Send;
}

/// Camera viewing secrets (ADR 0064). A secret's value never arrives here: only its SHA-256 fingerprint.
pub trait StreamAccessRepo {
    /// Records a viewing token for an account (valid [`crate::stream_access::VIEWING_TTL_MINUTES`] minutes).
    fn create_viewing(
        &self,
        user: UserId,
        hash: &[u8],
        prefix: &str,
        now: DateTime<Utc>,
    ) -> impl Future<Output = RepoResult<StreamAccess>> + Send;
    /// Records a personal stream credential for an account.
    fn create_credential(
        &self,
        user: UserId,
        new: &NewStreamCredential,
        hash: &[u8],
        prefix: &str,
        now: DateTime<Utc>,
    ) -> impl Future<Output = RepoResult<StreamAccess>> + Send;
    /// The secret of this kind with this fingerprint if it is **active** at `now` (neither revoked nor expired) and its
    /// account is active.
    fn find_active(
        &self,
        kind: StreamAccessKind,
        hash: &[u8],
        now: DateTime<Utc>,
    ) -> impl Future<Output = RepoResult<Option<ActiveStreamAccess>>> + Send;
    /// Notes the usage; writes at most once a minute per secret.
    fn touch(
        &self,
        id: StreamAccessId,
        now: DateTime<Utc>,
    ) -> impl Future<Output = RepoResult<()>> + Send;
    /// The credentials of an account, most recent first (active, expired and revoked). Viewing tokens are not listed.
    fn list_credentials(
        &self,
        user: UserId,
    ) -> impl Future<Output = RepoResult<Vec<StreamAccess>>> + Send;
    /// Revokes a credential of this account; `true` if it was active (another account's credential is never touched).
    fn revoke(
        &self,
        id: StreamAccessId,
        user: UserId,
        now: DateTime<Utc>,
    ) -> impl Future<Output = RepoResult<bool>> + Send;
    /// Deletes the viewing tokens that expired before `before`; returns how many.
    fn purge_expired_viewing(
        &self,
        before: DateTime<Utc>,
    ) -> impl Future<Output = RepoResult<u64>> + Send;
}

/// MCP audit log.
pub trait McpAuditRepo {
    /// Writes a line and returns its number. An action that changes something writes its line **before** running, with
    /// the outcome `pending`, then closes it with [`McpAuditRepo::complete`].
    fn record(
        &self,
        entry: &NewAuditEntry,
        now: DateTime<Utc>,
    ) -> impl Future<Output = RepoResult<i64>> + Send;
    /// Sets the outcome and detail of a line written as `pending`.
    fn complete(
        &self,
        id: i64,
        outcome: &str,
        detail: &str,
    ) -> impl Future<Output = RepoResult<()>> + Send;
    /// Most recent first, of a single type if `kind` is given; `limit` is bounded by the caller.
    fn recent(
        &self,
        kind: Option<AuditKind>,
        limit: i64,
    ) -> impl Future<Output = RepoResult<Vec<McpAuditEntry>>> + Send;
    /// Deletes what is older than `before`; returns the number of rows.
    fn purge_before(&self, before: DateTime<Utc>) -> impl Future<Output = RepoResult<u64>> + Send;
}
