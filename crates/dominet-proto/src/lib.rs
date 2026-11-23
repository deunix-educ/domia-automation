// SPDX-License-Identifier: AGPL-3.0-or-later
//! DTOs shared between the server and the WASM front end (and, eventually, the Android contract).
//!
//! No logic here: `serde` structures and their field names, which act as the contract.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RoleDto {
    Admin,
    User,
}

/// Account as shown to the interface: never a password or a hash.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UserDto {
    pub id: i64,
    pub username: String,
    pub display_name: String,
    pub email: String,
    pub mobile: String,
    pub role: RoleDto,
    pub is_active: bool,
    /// RFC 3339 (UTC), absent as long as the account has never logged in.
    pub last_login_at: Option<String>,
}

#[derive(Clone, PartialEq, Eq, Deserialize)]
pub struct LoginRequest {
    pub username: String,
    pub password: String,
}

// `Debug` must never show the password.
impl std::fmt::Debug for LoginRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LoginRequest")
            .field("username", &self.username)
            .finish_non_exhaustive()
    }
}

#[derive(Clone, PartialEq, Eq, Deserialize)]
pub struct ChangePasswordRequest {
    pub current_password: String,
    pub new_password: String,
}

impl std::fmt::Debug for ChangePasswordRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ChangePasswordRequest")
            .finish_non_exhaustive()
    }
}

/// Result of a login attempt from the interface (expected refusals are values, not errors).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum LoginOutcome {
    Success(UserDto),
    /// Unknown name, wrong password or deactivated account: same answer so as to reveal nothing.
    InvalidCredentials,
    Locked {
        retry_after_s: u64,
    },
}

/// Reason a new password is refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PasswordProblem {
    TooShort { min: u32 },
    TooLong { max: u32 },
    AllDigits,
    SameAsUsername,
}

/// Result of a password change from the interface.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ChangePasswordOutcome {
    /// Password changed; all the sessions of the account are closed.
    Changed,
    WrongCurrent,
    Rejected(PasswordProblem),
    /// Too many recent attempts: same lock-out as the login.
    Locked {
        retry_after_s: u64,
    },
}

/// Subject a browser can subscribe to on `/ws/notifications/`: `notifications`, `system`, `device:<id>`,
/// `camera:<uuid>`. The same notation is used on the wire, in the server's hub and in the browser.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Topic {
    /// Messages for the interface (all accounts): always subscribed to at opening.
    Notifications,
    /// Machine statistics (R2, lot 3).
    System,
    /// Number of ongoing alerts (R4, lot 6): sidebar badge and alerts page.
    Alerts,
    /// State of a device (strictly positive identifier).
    Device(i64),
    /// Events of an Android device (R5): presence, SMS, calls, acknowledgements. Reserved to its owner and to administrators.
    Android(i64),
    /// Progress of a message being sent (R5 lot 5): reserved to the sender, the recipients and administrators.
    Message(i64),
    /// Results of the ONVIF discovery (R6 lot 3): reserved to administrators.
    Discovery,
    /// Events of a camera (R6): refused as long as surveillance does not exist.
    Camera(uuid::Uuid),
}

impl std::fmt::Display for Topic {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Notifications => f.write_str("notifications"),
            Self::System => f.write_str("system"),
            Self::Alerts => f.write_str("alerts"),
            Self::Device(id) => write!(f, "device:{id}"),
            Self::Android(id) => write!(f, "android:{id}"),
            Self::Message(id) => write!(f, "message:{id}"),
            Self::Discovery => f.write_str("discovery"),
            Self::Camera(uuid) => write!(f, "camera:{}", uuid.hyphenated()),
        }
    }
}

/// Text that is not a valid subject.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvalidTopic;

impl std::str::FromStr for Topic {
    type Err = InvalidTopic;

    /// Strict: canonical form only (`device:7`, not `device:07` nor `Device:7` nor a space), so that two spellings
    /// of the same subject cannot create two subscriptions.
    fn from_str(s: &str) -> Result<Self, InvalidTopic> {
        let topic = match s {
            "notifications" => Self::Notifications,
            "system" => Self::System,
            "alerts" => Self::Alerts,
            "discovery" => Self::Discovery,
            _ => match s.split_once(':') {
                Some(("device", id)) => Self::Device(id.parse().map_err(|_| InvalidTopic)?),
                Some(("android", id)) => Self::Android(id.parse().map_err(|_| InvalidTopic)?),
                Some(("message", id)) => Self::Message(id.parse().map_err(|_| InvalidTopic)?),
                Some(("camera", uuid)) => {
                    Self::Camera(uuid::Uuid::parse_str(uuid).map_err(|_| InvalidTopic)?)
                }
                _ => return Err(InvalidTopic),
            },
        };
        if matches!(topic, Self::Device(id) | Self::Android(id) | Self::Message(id) if id <= 0)
            || topic.to_string() != s
        {
            return Err(InvalidTopic);
        }
        Ok(topic)
    }
}

/// State of a device as broadcast with each state message (contract of appendix A.1, taken from the reference implementation).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DeviceStateDto {
    pub device_id: i64,
    pub friendly_name: String,
    /// JSON object of the properties (`temperature`, `state`, `brightness`...).
    pub state: serde_json::Value,
    pub available: bool,
}

/// Processor: overall load and number of logical cores.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct CpuStats {
    pub percent: f64,
    pub count: u32,
}

/// Memory (sizes are in GiB, rounded to one decimal, like the reference implementation).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct MemoryStats {
    pub percent: f64,
    pub used_gb: f64,
    pub total_gb: f64,
    pub available_gb: f64,
}

/// Disk of the root `/`.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct DiskStats {
    pub percent: f64,
    pub used_gb: f64,
    pub total_gb: f64,
    pub free_gb: f64,
}

/// Machine resources (subject `system`, `GET /api/system-stats/`). Format of the reference implementation (`cpu`, `memory`,
/// `disk`, `updated_at`) extended with `temperature`; `disk` and `temperature` are `null` when the machine does not
/// expose them (container without a sensor, root not found): better an absent value than a wrong one.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct SystemStatsDto {
    pub cpu: CpuStats,
    pub memory: MemoryStats,
    pub disk: Option<DiskStats>,
    /// Degrees Celsius of the processor (or of the hottest sensor otherwise).
    pub temperature: Option<f64>,
    /// Unix seconds (decimal) of the measurement.
    pub updated_at: f64,
}

/// SMS as shown to the interface (fields of `to_dict` of the reference implementation).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SmsDto {
    pub id: i64,
    #[serde(default)]
    pub device_id: i64,
    /// `incoming` or `outgoing`.
    pub direction: String,
    pub phone_number: String,
    pub contact_name: String,
    pub body: String,
    pub is_read: bool,
    /// RFC 3339 (UTC).
    pub sent_at: String,
    /// Row of the sending queue of an outgoing SMS (this is the `message_id` of `sms_sent_ack`).
    #[serde(default)]
    pub outbox_id: Option<i64>,
    /// Sending state of an outgoing SMS: `pending`, `sent` or `failed`; empty for a received SMS.
    #[serde(default)]
    pub delivery: String,
    /// Reason an outgoing SMS failed.
    #[serde(default)]
    pub error: String,
}

/// Entry of the call log (fields of `to_dict` of the reference implementation).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CallDto {
    pub id: i64,
    #[serde(default)]
    pub device_id: i64,
    /// `incoming`, `outgoing` or `missed`.
    pub direction: String,
    pub phone_number: String,
    pub contact_name: String,
    pub started_at: String,
    pub ended_at: Option<String>,
    /// Seconds.
    pub duration: i32,
}

/// What happens on an Android device (subject `android:<id>`), in the format of the reference implementation's browser messages.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AndroidEvent {
    /// The device connects or disconnects; the last state is kept by the hub.
    DeviceStatus {
        is_online: bool,
    },
    SmsReceived {
        sms: SmsDto,
    },
    /// An SMS has just been handed to the device from a browser (state `pending` until the acknowledgement).
    SmsQueued {
        sms: SmsDto,
    },
    CallEvent {
        call: CallDto,
        state: String,
    },
    /// Acknowledgement of an SMS send: `message_id` is the identifier of the sending queue (that of `send_sms`).
    SmsSentAck {
        message_id: i64,
        success: bool,
        error: String,
    },
}

/// Android device as shown to the interface: never its token.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AndroidDeviceDto {
    pub id: i64,
    pub name: String,
    pub phone_number: String,
    pub owner: String,
    pub is_online: bool,
    /// RFC 3339 (UTC), empty as long as the device has never connected.
    pub last_seen: String,
}

/// Android dashboard: devices and recent activity (10 SMS and 10 calls at most).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AndroidOverviewDto {
    pub devices: Vec<AndroidDeviceDto>,
    pub recent_sms: Vec<SmsDto>,
    pub recent_calls: Vec<CallDto>,
    pub unread: u32,
}

/// SMS page of a device (the last 100); `device` is absent when the account has no device.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AndroidSmsPageDto {
    pub device: Option<AndroidDeviceDto>,
    pub devices: Vec<AndroidDeviceDto>,
    pub sms: Vec<SmsDto>,
}

/// Call log of a device (the last 100).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AndroidCallsPageDto {
    pub device: Option<AndroidDeviceDto>,
    pub devices: Vec<AndroidDeviceDto>,
    pub calls: Vec<CallDto>,
}

/// Registration of a device.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AndroidDeviceInput {
    pub name: String,
    pub phone_number: String,
}

/// A device that has just been created: its token is shown **only here**, once.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NewAndroidDeviceDto {
    pub device: AndroidDeviceDto,
    pub token: String,
}

// `Debug` must never show the token.
impl std::fmt::Debug for NewAndroidDeviceDto {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NewAndroidDeviceDto")
            .field("device", &self.device)
            .finish_non_exhaustive()
    }
}

/// Result of sending an SMS from the browser.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SendSmsOutcome {
    /// Handed to the device; the acknowledgement will arrive live.
    Queued(SmsDto),
    Invalid(Vec<FieldProblem>),
    /// The device is not connected: nothing was recorded.
    Offline,
    /// Unknown device, or not on the account.
    NotFound,
}

// ------------------------------------------------------------------ messaging (R5 lot 5)

/// A person who can be chosen as a message recipient.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecipientChoiceDto {
    pub id: i64,
    pub username: String,
    /// First and last name, or the username.
    pub display_name: String,
    pub has_email: bool,
    pub has_mobile: bool,
}

/// Data of the composition form.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ComposeFormDto {
    pub users: Vec<RecipientChoiceDto>,
    /// Missing sending settings (no SMTP server, no Android device online): simple information before sending.
    pub email_ready: bool,
    pub android_online: bool,
}

/// What the administrator enters: `channel` is `email`, `sms` or `both`; `audience` is `all`, `staff` or empty (choice per account).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ComposeInput {
    pub subject: String,
    pub body: String,
    pub channel: String,
    pub audience: String,
    pub users: Vec<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ComposeOutcome {
    /// Message recorded, sending in progress: `id` is that of the detail page.
    Sent {
        id: i64,
        recipients: u32,
    },
    Invalid(Vec<FieldProblem>),
}

/// Message in a list (sent or inbox).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MessageRowDto {
    pub id: i64,
    pub subject: String,
    pub sender: String,
    pub channel: String,
    pub status: String,
    pub recipients: i32,
    /// RFC 3339 (UTC).
    pub created_at: String,
    /// Statuses of the logged-in account for this message (inbox); empty elsewhere.
    pub my_email: String,
    pub my_sms: String,
}

/// Sending of a message to a recipient.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeliveryDto {
    pub id: i64,
    pub recipient: String,
    /// `pending`, `sent`, `failed` or `na`.
    pub email: String,
    pub sms: String,
    pub error: String,
}

/// Detail page: the full table (`deliveries`) for the sender and administrators, otherwise only `mine`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MessageDetailDto {
    pub message: MessageRowDto,
    pub body: String,
    pub full_table: bool,
    pub deliveries: Vec<DeliveryDto>,
    pub mine: Option<DeliveryDto>,
}

/// Why a subscription request is refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RefusalReason {
    /// Badly written subject.
    Invalid,
    /// Subject not allowed for this account (or not yet available).
    Forbidden,
    /// Too many subscriptions on this connection.
    Limit,
}

/// Message from the server to the browser on `/ws/notifications/` (one JSON value per text frame).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum WsServerMsg {
    /// First frame after opening: the connection is authenticated.
    Hello { username: String },
    /// Heartbeat: the browser must answer `pong` before the next one, otherwise the connection is closed.
    Ping,
    /// Message to show to the user.
    Notification {
        level: NotificationLevel,
        text: String,
    },
    /// State of a device (subject `device:<id>`); the last known state is sent upon subscription.
    DeviceState(DeviceStateDto),
    /// Machine resources (subject `system`), every few seconds.
    SystemStats(SystemStatsDto),
    /// Number of ongoing alerts (subject `alerts`); the last one is kept.
    AlertCount { firing: u32 },
    /// Event of an Android device (subject `android:<id>`).
    Android { device_id: i64, event: AndroidEvent },
    /// A message send has progressed (subject `message:<id>`): the detail page reloads its statuses. `complete`: nothing
    /// is pending any more. The last event is kept by the hub.
    MessageProgress { message_id: i64, complete: bool },
    /// Progress of the ONVIF discovery (subject `discovery`, administrators).
    Discovery { event: DiscoveryEvent },
    /// Subscription taken into account (or already active).
    Subscribed { topic: String },
    /// Subscription refused.
    Refused {
        topic: String,
        reason: RefusalReason,
    },
}

/// An ONVIF device that answered the discovery.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiscoveredDto {
    pub ip: String,
    pub port: u16,
    pub name: String,
    pub hardware: String,
    /// A registered camera already has this address.
    pub known: bool,
}

/// Step of the ONVIF discovery.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DiscoveryEvent {
    /// A search starts: the list restarts from zero.
    Scanning,
    Found {
        device: DiscoveredDto,
    },
    /// End of the listening.
    Done,
    /// The search failed (message without a secret).
    Failed {
        message: String,
    },
}

/// State of the discovery page, rebuilt from the events.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct DiscoveryView {
    pub scanning: bool,
    pub done: bool,
    pub found: Vec<DiscoveredDto>,
    pub error: Option<String>,
}

impl DiscoveryView {
    pub fn apply(&mut self, event: DiscoveryEvent) {
        match event {
            DiscoveryEvent::Scanning => {
                *self = Self {
                    scanning: true,
                    ..Self::default()
                };
            }
            DiscoveryEvent::Found { device } => {
                if !self.found.iter().any(|d| d.ip == device.ip) {
                    self.found.push(device);
                }
            }
            DiscoveryEvent::Done => {
                self.scanning = false;
                self.done = true;
            }
            DiscoveryEvent::Failed { message } => {
                self.scanning = false;
                self.error = Some(message);
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum NotificationLevel {
    Info,
    Warning,
    Error,
}

/// Message from the browser to the server.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum WsClientMsg {
    Pong,
    /// Subscribe to a subject (canonical form, see [`Topic`]).
    Subscribe {
        topic: String,
    },
    Unsubscribe {
        topic: String,
    },
}

/// Result of broadcasting a message to all the connected users.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BroadcastOutcome {
    /// Number of open connections that received the message.
    Sent { receivers: u32 },
    /// Empty or too long text.
    Invalid,
}

/// Result of injecting a test device state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum InjectOutcome {
    /// Number of subscribers that received it.
    Sent { receivers: u32 },
    /// No device with this number.
    InvalidDevice,
    /// Not a JSON object, or too big (4 KiB at most).
    InvalidState,
}

/// An account as shown and edited in the administration (never a password or a hash).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UserAdminDto {
    pub id: i64,
    pub username: String,
    pub first_name: String,
    pub last_name: String,
    pub email: String,
    pub mobile: String,
    pub role: RoleDto,
    pub is_active: bool,
    pub last_login_at: Option<String>,
}

/// Creation (`id` absent) or modification of an account. `username` is ignored on modification (immutable). `password`:
/// mandatory at creation, optional afterwards (non-empty = reset).
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UserInput {
    pub id: Option<i64>,
    pub username: String,
    pub first_name: String,
    pub last_name: String,
    pub email: String,
    pub mobile: String,
    pub role: String,
    pub is_active: bool,
    pub password: String,
}

// `Debug` must never show the password.
impl std::fmt::Debug for UserInput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("UserInput")
            .field("id", &self.id)
            .field("username", &self.username)
            .finish_non_exhaustive()
    }
}

/// A location, as shown and edited in the administration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LocationDto {
    pub id: i64,
    pub name: String,
    pub description: String,
    pub active: bool,
}

/// Creation (`id` absent) or replacement (`id` present) of a location.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LocationInput {
    pub id: Option<i64>,
    pub name: String,
    pub description: String,
    pub active: bool,
}

/// Prometheus alert rule, as shown and edited in the administration (R7, ADR 0052).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PrometheusRuleDto {
    pub id: i64,
    pub name: String,
    pub group: String,
    pub severity: String,
    pub expr: String,
    pub for_duration: String,
    pub summary: String,
    pub description: String,
    pub eval_interval: String,
    pub active: bool,
}

/// Creation (`id` absent) or replacement (`id` present) of a Prometheus rule.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PrometheusRuleInput {
    pub id: Option<i64>,
    pub name: String,
    pub group: String,
    pub severity: String,
    pub expr: String,
    pub for_duration: String,
    pub summary: String,
    pub description: String,
    pub eval_interval: String,
    pub active: bool,
}

/// Grafana alert rule, as shown and edited in the administration (R7, ADR 0052).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GrafanaRuleDto {
    pub id: i64,
    pub uid: String,
    pub title: String,
    pub folder: String,
    pub group: String,
    pub group_interval: String,
    pub datasource_uid: String,
    pub expr: String,
    pub query_window: i32,
    pub for_duration: String,
    pub severity: String,
    pub summary: String,
    pub description: String,
    pub no_data_state: String,
    pub exec_err_state: String,
    pub active: bool,
}

/// Creation (`id` absent) or replacement (`id` present) of a Grafana rule. Empty `uid`: derived from the title.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GrafanaRuleInput {
    pub id: Option<i64>,
    pub uid: String,
    pub title: String,
    pub folder: String,
    pub group: String,
    pub group_interval: String,
    pub datasource_uid: String,
    pub expr: String,
    pub query_window: i32,
    pub for_duration: String,
    pub severity: String,
    pub summary: String,
    pub description: String,
    pub no_data_state: String,
    pub exec_err_state: String,
    pub active: bool,
}

/// Result of writing the rule files and reloading a service.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuleSyncDto {
    /// `disabled` (no file configured), `ok`, `no_url` (file written, reload to be done by hand),
    /// `reload_failed` (file written, service unreachable or refusal) or `failed` (writing impossible).
    pub state: String,
    pub groups: u32,
    pub detail: String,
}

/// A camera, as shown and edited in the administration (R6, ADR 0043). No secret: only whether it is set.
// Independent flags of the form.
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CameraDto {
    pub id: i64,
    /// Public identifier (32 hexadecimal digits): name of the MediaMTX path.
    pub uuid: String,
    pub name: String,
    pub location_id: Option<i64>,
    pub active: bool,
    pub priority: i32,
    pub notes: String,
    pub proto: String,
    pub ip: String,
    pub port: i32,
    pub path: String,
    pub rtsp_user: String,
    pub has_rtsp_pass: bool,
    pub fps: i32,
    pub rtsp_any_port: bool,
    pub rtsp_transport_tcp: bool,
    pub recording_mode: String,
    pub night_mode_api: String,
    pub web_port: i32,
    pub has_web_api_key: bool,
    pub web_user: String,
    pub has_web_pass: bool,
    pub onvif: bool,
    pub onvif_port: i32,
    pub onvif_user: String,
    pub has_onvif_pass: bool,
    pub ptz: bool,
    pub yolo_enabled: bool,
    /// Comma-separated classes; empty = all.
    pub yolo_classes: String,
    /// Minimum confidence in percent (0 to 100).
    pub yolo_min_confidence_pct: i32,
    pub manufacturer: String,
    pub model: String,
    pub firmware: String,
    /// Last connection (RFC 3339), if known.
    pub last_seen: Option<String>,
    /// Token of the main ONVIF profile (learned by the synchronisation, editable).
    #[serde(default)]
    pub main_token: String,
}

/// Creation (`id` absent) or replacement (`id` present) of a camera. A secret left empty stays unchanged.
// Independent flags of the form.
#[allow(clippy::struct_excessive_bools)]
#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub struct CameraInput {
    pub id: Option<i64>,
    pub name: String,
    pub location_id: Option<i64>,
    pub active: bool,
    pub priority: i32,
    pub notes: String,
    pub proto: String,
    pub ip: String,
    pub port: i32,
    pub path: String,
    pub rtsp_user: String,
    pub rtsp_pass: String,
    pub fps: i32,
    pub rtsp_any_port: bool,
    pub rtsp_transport_tcp: bool,
    pub recording_mode: String,
    pub night_mode_api: String,
    pub web_port: i32,
    pub web_api_key: String,
    pub web_user: String,
    pub web_pass: String,
    pub onvif: bool,
    pub onvif_port: i32,
    pub onvif_user: String,
    pub onvif_pass: String,
    /// The camera is PTZ-controllable (ticked by hand; the first ONVIF synchronisation suggests it).
    pub ptz: bool,
    pub yolo_enabled: bool,
    pub yolo_classes: String,
    pub yolo_min_confidence_pct: i32,
    /// Token of the main ONVIF profile; absent or empty: the value learned by the synchronisation is kept.
    #[serde(default)]
    pub main_token: Option<String>,
}

// `Debug` must never show a password or an API key.
impl std::fmt::Debug for CameraInput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CameraInput")
            .field("id", &self.id)
            .field("name", &self.name)
            .finish_non_exhaustive()
    }
}

/// A camera on the viewing page (any logged-in account): nothing of the stream configuration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CameraCardDto {
    pub uuid: String,
    pub name: String,
    pub location: String,
    pub recording_mode: String,
    pub ptz: bool,
    pub last_seen: Option<String>,
}

/// Outcome of a MediaMTX synchronisation requested from the administration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum MediaMtxSyncDto {
    /// `MEDIAMTX_API_URL` is not set.
    Disabled,
    /// MediaMTX does not answer (message already cleaned of any secret).
    Unreachable(String),
    Done {
        added: u32,
        replaced: u32,
        removed: u32,
        /// (camera, message) for what failed.
        failed: Vec<(String, String)>,
    },
}

/// Adding a camera by its IP address (R6 lot 3). The credentials are used for the stream and for ONVIF.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AddCameraByIp {
    pub ip: String,
    /// Known ONVIF port; absent: the usual ports are tried.
    pub onvif_port: Option<u16>,
    pub user: String,
    pub pass: String,
}

// `Debug` must never show the password.
impl std::fmt::Debug for AddCameraByIp {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AddCameraByIp")
            .field("ip", &self.ip)
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum AddByIpOutcome {
    /// `created`: false if a camera already had this address.
    Added {
        id: i64,
        name: String,
        created: bool,
        model: String,
    },
    Invalid(Vec<FieldProblem>),
    /// No ONVIF service on the usual ports.
    NoOnvif,
}

/// Result of an ONVIF synchronisation requested from the administration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum OnvifSyncOutcome {
    Done {
        manufacturer: String,
        model: String,
        firmware: String,
        main_token: String,
        ptz: bool,
        /// Stream URL, password masked.
        stream_uri: String,
    },
    /// ONVIF is not enabled on this camera.
    Disabled,
    /// ONVIF credentials refused by the camera.
    NotAuthorized,
    /// Camera unreachable or unreadable response (message without a secret).
    Failed(String),
    NotFound,
}

/// An indexed snapshot, as the gallery shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SnapshotDto {
    pub id: i64,
    pub camera_uuid: String,
    pub camera_name: String,
    /// RFC 3339.
    pub taken_at: String,
    pub size_bytes: i32,
}

/// A camera event (motion or object detection), as the events page shows it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CameraEventDto {
    /// `motion` or `detection`.
    pub kind: String,
    /// RFC 3339, server time.
    pub at: String,
    pub camera_uuid: String,
    pub camera_name: String,
    /// Share of the image that moved, in percent (motion).
    pub motion_pct: Option<f64>,
    /// Recognised classes, comma-separated (detection).
    pub classes: String,
    /// Identifier of the detection when an annotated image exists.
    pub frame_id: Option<i64>,
}

/// Global setting of the object detector.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DetectorDto {
    pub enabled: bool,
    pub save_frames: bool,
}

/// A recorded segment, as the recordings page shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecordingDto {
    pub id: i64,
    /// RFC 3339.
    pub started_at: String,
    pub duration_ms: Option<i64>,
    /// `continuous`, `motion` or `manual`.
    pub event_type: String,
    /// `pending`, `uploading`, `done` or `error`.
    pub status: String,
    pub size_bytes: Option<i64>,
    pub error: String,
}

/// A camera's recordings for one day.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecordingsDayDto {
    pub camera_name: String,
    pub recording_mode: String,
    /// Manual recording in progress (`None`: camera not in manual mode, or MediaMTX does not say).
    pub manual_on: Option<bool>,
    pub items: Vec<RecordingDto>,
}

/// Result of starting / stopping the manual recording.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ManualRecordOutcome {
    Done {
        on: bool,
    },
    NotFound,
    NotManual,
    /// MediaMTX absent or down.
    Unavailable(String),
}

/// Result of a capture requested from the live page (administrators).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum CaptureOutcome {
    Saved(SnapshotDto),
    /// No recent image: the `video` role does not decode this camera (camera off, MediaMTX without a stream...).
    NoImage,
    NotFound,
}

/// An MQTT source, as shown and edited in the administration (R3, ADR 0028).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MqttSourceDto {
    pub id: i64,
    pub name: String,
    pub topic_pattern: String,
    pub qos: u8,
    pub active: bool,
    pub log_messages: bool,
    pub auto_device: bool,
    /// Technical identifier of the type of device created (`sensor`, `switch`...).
    pub auto_device_type: String,
    pub base_topic: String,
    pub handler_code: String,
    pub description: String,
}

/// Creation (`id` absent) or replacement (`id` present) of an MQTT source.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MqttSourceInput {
    pub id: Option<i64>,
    pub name: String,
    pub topic_pattern: String,
    pub qos: u8,
    pub active: bool,
    pub log_messages: bool,
    pub auto_device: bool,
    pub auto_device_type: String,
    pub base_topic: String,
    pub handler_code: String,
    pub description: String,
}

/// What prevents an administration operation although the data is valid.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Blocked {
    /// At least one active administrator must remain.
    LastAdmin,
    /// An administrator cannot delete, deactivate or demote themselves.
    Yourself,
    /// Their own password is changed in "My account" (the session would be closed in the middle of the page).
    OwnPassword,
}

/// Reason a form field is refused.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProblemKind {
    Required,
    TooLong {
        max: u32,
    },
    /// Format or value not allowed (the field shows a hint about the expected form).
    Invalid,
    OutOfRange {
        min: i64,
        max: i64,
    },
    /// Value already taken (identifier of an account).
    Taken,
    /// The location of a widget overlaps another widget of the dashboard.
    Overlap,
    /// Password refused by the policy.
    Password(PasswordProblem),
    /// Script that does not compile: the parser's message (line and column included).
    Syntax(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FieldProblem {
    pub field: String,
    pub kind: ProblemKind,
}

/// A navigation entry as edited in the administration (the icon is a *name* there, not an SVG).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NavbarAdminDto {
    pub id: i64,
    pub icon_name: String,
    pub label: String,
    pub url: String,
    /// `_self` or `_blank`.
    pub url_target: String,
    pub admin_only: bool,
    /// `top` or `bottom`.
    pub position: String,
    pub sort_order: i32,
}

/// Creation (`id` absent) or replacement (`id` present) of a navigation entry. `sort_order` is text:
/// the interface sends what the user typed and the server says what is wrong.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NavbarInput {
    pub id: Option<i64>,
    pub icon_name: String,
    pub label: String,
    pub url: String,
    pub url_target: String,
    pub admin_only: bool,
    pub position: String,
    pub sort_order: String,
}

/// Result of saving a setting.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SettingOutcome {
    Saved(SettingDto),
    /// Value incompatible with the setting's type.
    Invalid,
    /// A secret cannot be saved without `DOMINET_MASTER_KEY`.
    MasterKeyMissing,
    /// Key absent from the registry.
    UnknownKey,
}

/// Result of a save from the administration (expected refusals are values).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SaveOutcome<T> {
    Saved(T),
    Invalid(Vec<FieldProblem>),
    /// The item was deleted in the meantime.
    NotFound,
    Blocked(Blocked),
}

impl<T> SaveOutcome<T> {
    /// Converts the saved item (for instance a domain object into its DTO), the refusals unchanged.
    #[must_use]
    pub fn map<U>(self, f: impl FnOnce(T) -> U) -> SaveOutcome<U> {
        match self {
            Self::Saved(item) => SaveOutcome::Saved(f(item)),
            Self::Invalid(list) => SaveOutcome::Invalid(list),
            Self::NotFound => SaveOutcome::NotFound,
            Self::Blocked(b) => SaveOutcome::Blocked(b),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DeleteOutcome {
    Deleted,
    NotFound,
    /// Still referenced elsewhere (foreign-key constraint).
    InUse,
    Blocked(Blocked),
}

/// A navigation entry, ready to display: the icon is already an SVG rendered on the server side.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NavItemDto {
    pub label: String,
    pub url: String,
    pub url_target: String,
    pub icon_svg: String,
}

/// What the layout needs: the logged-in account and the navigation entries **it is allowed to see**.
/// For an anonymous visitor, everything is empty (the navigation is not disclosed).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct LayoutDto {
    pub user: Option<UserDto>,
    pub nav_top: Vec<NavItemDto>,
    pub nav_bottom: Vec<NavItemDto>,
}

/// A dashboard in the selector.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DashboardSummaryDto {
    pub id: i64,
    pub name: String,
    pub is_default: bool,
}

/// A widget prepared for display. The `content_html` and `iframe_src` fields are computed by the server: the HTML is
/// **already sanitised** and the iframe address **already verified** (allowed origin); the browser has no decision of
/// security to make.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WidgetDto {
    pub id: i64,
    /// `camera`, `device`, `scene`, `grafana`, `text` or `grille`.
    pub widget_type: String,
    pub title: String,
    /// Requested presentation (`""` = automatic), values of the reference implementation.
    pub presentation: String,
    pub device_id: Option<i64>,
    pub col: i16,
    pub row: i16,
    pub width: i16,
    pub height: i16,
    /// Sanitised HTML of the `text` widget.
    pub content_html: String,
    /// Address to load in the iframe of a Grafana widget, if it is allowed.
    pub iframe_src: Option<String>,
    /// Why there is no iframe: `not_configured` or `forbidden` (empty when `iframe_src` exists).
    pub iframe_problem: String,
    /// Last known state of the device at render time (real time takes over afterwards).
    pub device: Option<DeviceStateDto>,
    /// What the devices table knows about the device (type, labels, hidden keys, possible commands).
    pub device_meta: Option<DeviceMetaDto>,
    /// The scene of a `scene` widget (absent if it was deleted or if it is not a scene widget).
    pub scene: Option<SceneBriefDto>,
    /// The camera of a `camera` widget (absent if unknown or inactive).
    pub camera: Option<CameraLiveDto>,
}

/// What is needed to display a camera's live view: never a source stream address or credentials.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CameraLiveDto {
    /// Public identifier (32 hexadecimal digits).
    pub uuid: String,
    pub name: String,
    /// WHEP address of MediaMTX, adapted to the host the browser arrived through.
    pub whep_url: String,
    pub ptz: bool,
    pub recording_mode: String,
    /// A day / night method is configured (the buttons only make sense in that case; they are only shown to administrators).
    pub night: bool,
}

/// Result of a day / night / light toggle requested from the live page (administrators).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum NightOutcomeDto {
    /// `details`: what the camera did, in JSON (the reference's shape: `mode`, `was`, `value`, `brightness`...).
    Done { action: String, details: String },
    /// No day / night method is configured for this camera.
    NotConfigured,
    /// Unknown or inactive camera.
    NotFound,
    /// Action other than `daynight`, `ircut`, `light`.
    InvalidAction,
    /// The camera did not obey (message without a secret).
    Failed(String),
}

/// What a scene widget shows of its scene.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SceneBriefDto {
    pub id: i64,
    pub name: String,
    pub icon: String,
    pub description: String,
    pub actions: u32,
    pub active: bool,
}

/// Label of a state key (readable name and unit), as the interface shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LabelDto {
    pub key: String,
    pub name: String,
    pub unit: String,
}

/// Information about a device useful for displaying its state and its commands.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct DeviceMetaDto {
    /// Stored type (`sensor`, `light`...).
    pub device_type: String,
    pub labels: Vec<LabelDto>,
    /// State keys not to display.
    pub hidden_keys: Vec<String>,
    /// True if the server can publish commands (MQTT client started).
    pub commandable: bool,
}

/// A device as the administration shows and edits it (the key lists are comma-separated).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceAdminDto {
    pub id: i64,
    pub friendly_name: String,
    pub description: String,
    pub device_type: String,
    /// Group number, or empty text.
    pub group_id: String,
    /// Location number, or empty text.
    pub location_id: String,
    pub base_topic: String,
    pub active: bool,
    pub icon: String,
    pub image_url: String,
    pub tracked_keys: String,
    pub hidden_keys: String,
    pub grafana_dashboard: bool,
    pub vendor: String,
    pub model_id: String,
    pub ieee: String,
}

/// Creation (`id` absent) or replacement (`id` present) of a device: only the parameters, not the state or the discovery.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceInput {
    pub id: Option<i64>,
    pub friendly_name: String,
    pub description: String,
    pub device_type: String,
    pub group_id: String,
    pub location_id: String,
    pub base_topic: String,
    pub active: bool,
    pub icon: String,
    pub image_url: String,
    pub tracked_keys: String,
    pub hidden_keys: String,
    pub grafana_dashboard: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceGroupDto {
    pub id: i64,
    pub name: String,
    pub location_id: String,
    pub description: String,
    pub icon: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceGroupInput {
    pub id: Option<i64>,
    pub name: String,
    pub location_id: String,
    pub description: String,
    pub icon: String,
}

/// Label of a state key as edited in the administration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceLabelDto {
    pub id: i64,
    pub key: String,
    pub name: String,
    pub icon_name: String,
    pub unit: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceLabelInput {
    pub id: Option<i64>,
    pub key: String,
    pub name: String,
    pub icon_name: String,
    pub unit: String,
}

/// Choices of the drop-down lists of the forms (number, name).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct DeviceChoicesDto {
    pub groups: Vec<(i64, String)>,
    pub locations: Vec<(i64, String)>,
    pub devices: Vec<(i64, String)>,
    pub scenes: Vec<(i64, String)>,
}

/// A device on the devices page: identity, last known state and what is needed to display it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DeviceCardDto {
    pub id: i64,
    pub name: String,
    pub description: String,
    pub device_type: String,
    pub group: String,
    pub location: String,
    /// Last message received, in RFC 3339 (empty text if never).
    pub last_seen: String,
    /// Date of the uploaded image (seconds since 1970), for the address and the cache; `None` without an image.
    pub image_version: Option<i64>,
    pub state: DeviceStateDto,
    pub meta: DeviceMetaDto,
}

/// `/automation/` page: the filtered active devices, and what is needed to fill in the filters.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct DevicesPageDto {
    pub devices: Vec<DeviceCardDto>,
    pub groups: Vec<(i64, String)>,
    pub locations: Vec<(i64, String)>,
    /// Types present (stored identifiers).
    pub types: Vec<String>,
    pub permit_join: bool,
    pub commandable: bool,
}

/// History of a tracked key over the last 24 h.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SeriesDto {
    pub key: String,
    pub name: String,
    pub unit: String,
    /// (instant in seconds since 1970, value), from oldest to most recent.
    pub points: Vec<(i64, f64)>,
}

/// Page of a device.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DeviceDetailDto {
    pub card: DeviceCardDto,
    pub ieee: String,
    pub vendor: String,
    pub model_id: String,
    pub state_topic: String,
    pub command_topic: String,
    pub tracked_keys: Vec<String>,
    pub active: bool,
    pub series: Vec<SeriesDto>,
    /// Name of the Grafana dashboard file already generated for this device (admin), if there is one.
    pub grafana_uid: String,
}

/// A weather station as the administration shows it (latitude and longitude as text: entered and displayed as they are).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WeatherStationDto {
    pub id: i64,
    pub name: String,
    pub latitude: String,
    pub longitude: String,
    pub altitude: String,
    pub timezone: String,
    pub mqtt_topic_suffix: String,
    pub refresh_minutes: String,
    pub enabled: bool,
    pub description: String,
}

/// Creation (`id` absent) or replacement (`id` present) of a weather station; the numbers arrive as text and are read by the server.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WeatherStationInput {
    pub id: Option<i64>,
    pub name: String,
    pub latitude: String,
    pub longitude: String,
    pub altitude: String,
    pub timezone: String,
    pub mqtt_topic_suffix: String,
    pub refresh_minutes: String,
    pub enabled: bool,
    pub description: String,
}

/// Result of generating the Grafana dashboard of a device.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum GrafanaOutcome {
    /// File written (Grafana reloads it in about ten seconds).
    Written,
    /// The device has no tracked key: nothing to show.
    NoTrackedKeys,
    /// `GRAFANA_DASHBOARDS_DIR` is not set, or the folder is not writable.
    NotConfigured,
    NotFound,
}

/// Result of sending a command to a device.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CommandOutcome {
    /// Published on the broker.
    Sent,
    /// Unknown or inactive device.
    NotFound,
    /// Command that is not a reasonable JSON object.
    Invalid,
    /// No link with the broker (`mqtt` role absent, broker down, queue full).
    Unavailable,
}

/// A command of a scene, as shown.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SceneActionDto {
    pub device_id: i64,
    pub device: String,
    /// JSON command, compact.
    pub payload: String,
}

/// A scene with its commands in publication order (R4, ADR 0031).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SceneDto {
    pub id: i64,
    pub name: String,
    pub icon: String,
    pub description: String,
    /// Location number as text (`""` = none), like the other forms.
    pub location_id: String,
    pub location: String,
    pub active: bool,
    pub actions: Vec<SceneActionDto>,
}

/// Scene to create or replace, as entered. `actions`: list of `(device number, JSON command)` as text.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SceneInput {
    pub id: Option<i64>,
    pub name: String,
    #[serde(default)]
    pub icon: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub location_id: String,
    pub active: bool,
    #[serde(default)]
    pub actions: Vec<(String, String)>,
}

/// A scene command that did not go out.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SceneFailureDto {
    pub device: String,
    pub outcome: CommandOutcome,
}

/// Result of activating a scene: the commands are published in order, one that fails does not stop the following ones.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SceneOutcome {
    Activated {
        sent: u32,
        failed: Vec<SceneFailureDto>,
    },
    /// Unknown or inactive scene.
    NotFound,
}

/// The fields of an automation as the form enters them (all text: the server says what is wrong, field by
/// field). Those that do not concern the chosen type are ignored.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct AutomationFields {
    pub name: String,
    pub enabled: bool,
    pub description: String,
    /// `mqtt_message`, `device_state`, `schedule` or `solar`.
    pub trigger_type: String,
    pub topic: String,
    /// JSON object (optional).
    pub payload_filter: String,
    pub friendly_name: String,
    pub key: String,
    pub value: String,
    pub cron: String,
    pub event: String,
    pub latitude: String,
    pub longitude: String,
    pub condition_key: String,
    pub condition_value: String,
    /// `mqtt_publish`, `scene`, `notification` or `script`.
    pub action_type: String,
    pub action_topic: String,
    /// JSON object.
    pub action_payload: String,
    pub message: String,
    pub sms: bool,
    pub scene_id: String,
    pub script: String,
}

/// Creation (`id` absent) or replacement (`id` present) of an automation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AutomationInput {
    pub id: Option<i64>,
    pub fields: AutomationFields,
}

/// An automation with what the administration shows of it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AutomationDto {
    pub id: i64,
    pub fields: AutomationFields,
    /// Readable summary of the trigger and of the action.
    pub trigger_summary: String,
    pub action_summary: String,
    pub trigger_count: i32,
    /// RFC 3339, empty if never triggered.
    pub last_triggered: String,
    /// Next pass of the scheduler (time or sun), RFC 3339; empty otherwise.
    pub next_run: String,
}

/// A row of the automations' execution log.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AutomationRunDto {
    pub automation_id: i64,
    pub automation: String,
    /// RFC 3339.
    pub at: String,
    pub trigger: String,
    /// `ok`, `error`, `skipped` or `limited`.
    pub status: String,
    pub duration_ms: i32,
    pub message: String,
}

/// Result of a dry run of a script: nothing was published, enabled or saved.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DryRunDto {
    /// Syntax or execution error; empty if the script ended well.
    pub error: String,
    /// What the script would have done, in order.
    pub effects: Vec<String>,
    /// Lines of `print` and `log`.
    pub logs: Vec<String>,
    pub elapsed_ms: u64,
}

/// An alert as the `/alerts` page shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AlertDto {
    pub id: i64,
    /// Empty if the rule was deleted since.
    pub rule: String,
    pub device_id: i64,
    pub device: String,
    /// `info`, `warning` or `critical`.
    pub severity: String,
    /// `firing`, `resolved` or `acknowledged`.
    pub status: String,
    pub value: String,
    /// RFC 3339.
    pub fired_at: String,
    pub resolved_at: String,
    pub acknowledged_at: String,
    /// Username of whoever acknowledged.
    pub acknowledged_by: String,
}

/// The alerts page: the chosen filter, the number of ongoing alerts and the last 200 alerts.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct AlertsPageDto {
    pub status: String,
    pub firing: u32,
    pub alerts: Vec<AlertDto>,
    /// The platform's alerts (Prometheus and Grafana), read-only.
    pub platform: PlatformAlertsDto,
}

/// An ongoing alert at Prometheus or Grafana.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlatformAlertDto {
    /// `prometheus` or `grafana`.
    pub source: String,
    pub name: String,
    /// `info`, `warning` or `critical`.
    pub severity: String,
    pub summary: String,
    pub description: String,
    /// RFC 3339, empty if the source does not give it.
    pub since: String,
}

/// What the page knows about the platform: the state of each source (`off`: not monitored, `down`: unreachable, `ok`: read) and the
/// ongoing alerts, the most serious first.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct PlatformAlertsDto {
    pub prometheus: String,
    pub grafana: String,
    pub alerts: Vec<PlatformAlertDto>,
}

/// An alert rule as the administration shows and edits it (all text).
// Mirror of a form row: the channels and the activation are independent.
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct AlertRuleDto {
    pub id: i64,
    pub name: String,
    pub device_id: String,
    pub device: String,
    pub key: String,
    pub operator: String,
    pub threshold_num: String,
    pub threshold_str: String,
    pub severity: String,
    pub cooldown_minutes: String,
    pub notify_email: bool,
    pub notify_sms: bool,
    pub notify_android: bool,
    pub enabled: bool,
    pub description: String,
    /// Ongoing alerts of this rule.
    pub firing: u32,
}

/// Creation (`id` absent) or replacement of an alert rule.
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AlertRuleInput {
    pub id: Option<i64>,
    pub name: String,
    pub device_id: String,
    pub key: String,
    pub operator: String,
    pub threshold_num: String,
    pub threshold_str: String,
    pub severity: String,
    pub cooldown_minutes: String,
    pub notify_email: bool,
    pub notify_sms: bool,
    pub notify_android: bool,
    pub enabled: bool,
    pub description: String,
}

/// Result of acknowledging an alert.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AckOutcome {
    Acknowledged,
    /// Unknown alert, or already resolved / acknowledged.
    NotFiring,
}

/// A day of the energy summary as the page shows it (Wh).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnergyRowDto {
    pub date: String,
    pub solar_wh: i32,
    pub linky_wh: i32,
    pub pac_wh: i32,
    pub net_wh: i32,
}

/// A device of the summary with its last known state (real time takes over afterwards through `device:<id>`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EnergyLiveDto {
    pub id: i64,
    pub name: String,
    pub state: serde_json::Value,
    pub available: bool,
}

/// The energy page: days (oldest to most recent), an already computed SVG chart, and the summary's three devices.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct EnergyPageDto {
    pub days: u32,
    pub rows: Vec<EnergyRowDto>,
    /// Full SVG (computed on the server side, without script); empty without data.
    pub svg: String,
    pub linky: Option<EnergyLiveDto>,
    pub solar: Option<EnergyLiveDto>,
    pub pac: Option<EnergyLiveDto>,
}

/// What the home page shows: the account's dashboards, the open one and its widgets.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct DashboardPageDto {
    pub dashboards: Vec<DashboardSummaryDto>,
    pub current: Option<DashboardSummaryDto>,
    pub widgets: Vec<WidgetDto>,
}

/// A dashboard as the administration shows it (all accounts).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DashboardAdminDto {
    pub id: i64,
    pub name: String,
    /// Username of the owner.
    pub owner: String,
    pub is_default: bool,
}

/// Dashboard to create or rename. The owner is designated by their username (it no longer changes after creation);
/// `is_default` can only be enabled.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DashboardInput {
    pub id: Option<i64>,
    pub name: String,
    pub owner: String,
    pub is_default: bool,
}

/// A widget as the administration edits it: the numbers and the JSON travel as text (validation by the server).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WidgetAdminDto {
    pub id: i64,
    pub dashboard_id: i64,
    pub widget_type: String,
    pub title: String,
    pub presentation: String,
    pub device_id: String,
    pub scene_id: String,
    pub camera_uuid: String,
    /// JSON object, as text (`{}` if empty).
    pub config: String,
    pub content: String,
    pub col: i16,
    pub row: i16,
    pub width: i16,
    pub height: i16,
}

/// Widget to create or replace, as entered (all text: the server says what is wrong, field by field).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WidgetInput {
    pub id: Option<i64>,
    pub dashboard_id: i64,
    pub widget_type: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub presentation: String,
    #[serde(default)]
    pub device_id: String,
    #[serde(default)]
    pub scene_id: String,
    #[serde(default)]
    pub camera_uuid: String,
    #[serde(default)]
    pub config: String,
    #[serde(default)]
    pub content: String,
    #[serde(default)]
    pub col: String,
    #[serde(default)]
    pub row: String,
    #[serde(default)]
    pub width: String,
    #[serde(default)]
    pub height: String,
}

/// What the widgets administration page shows: the chosen dashboard (the requested one if it exists, otherwise the first) and its widgets.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct WidgetsPageDto {
    pub current: Option<i64>,
    pub widgets: Vec<WidgetAdminDto>,
}

/// Result of moving a widget.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MoveOutcome {
    Moved,
    /// The location overlaps another widget: the widget stays where it was.
    Overlap,
    /// Outside the grid.
    Invalid,
    /// The widget was deleted in the meantime.
    NotFound,
}

/// A setting as shown to the interface: a secret's value is **never** transmitted.
// Four independent flags: this is the intended shape of the JSON contract.
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SettingDto {
    pub key: String,
    /// `str`, `int`, `float` or `bool`.
    pub value_type: String,
    pub category: String,
    pub description: String,
    pub is_secret: bool,
    /// Displayable value (`********` for a secret that is set, empty if it is not).
    pub value: String,
    pub is_set: bool,
    /// True if the default value applies (nothing is saved in the database).
    pub from_default: bool,
    /// False if a saved secret cannot be decrypted (master key changed).
    pub readable: bool,
    pub updated_at: Option<String>,
}

/// New value of a setting (in clear text, including for a secret: write-only).
#[derive(Clone, PartialEq, Eq, Deserialize)]
pub struct SettingUpdate {
    pub value: String,
}

impl std::fmt::Debug for SettingUpdate {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SettingUpdate").finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_stats() -> SystemStatsDto {
        SystemStatsDto {
            cpu: CpuStats {
                percent: 12.5,
                count: 4,
            },
            memory: MemoryStats {
                percent: 40.0,
                used_gb: 3.2,
                total_gb: 8.0,
                available_gb: 4.8,
            },
            disk: Some(DiskStats {
                percent: 55.1,
                used_gb: 100.0,
                total_gb: 200.0,
                free_gb: 90.0,
            }),
            temperature: None,
            updated_at: 1_700_000_000.5,
        }
    }

    #[test]
    fn system_stats_keep_the_reference_keys_and_add_temperature() {
        let v = serde_json::to_value(sample_stats()).unwrap();
        let keys = |o: &serde_json::Value| {
            let mut k: Vec<_> = o.as_object().unwrap().keys().cloned().collect();
            k.sort();
            k
        };
        assert_eq!(
            keys(&v),
            ["cpu", "disk", "memory", "temperature", "updated_at"]
        );
        assert_eq!(keys(&v["cpu"]), ["count", "percent"]);
        assert_eq!(
            keys(&v["memory"]),
            ["available_gb", "percent", "total_gb", "used_gb"]
        );
        assert_eq!(
            keys(&v["disk"]),
            ["free_gb", "percent", "total_gb", "used_gb"]
        );
        assert!(v["temperature"].is_null());
    }

    #[test]
    fn system_stats_message_is_flat_and_round_trips() {
        let msg = WsServerMsg::SystemStats(sample_stats());
        let json = serde_json::to_value(&msg).unwrap();
        assert_eq!(json["type"], "system_stats");
        assert_eq!(json["cpu"]["count"], 4);
        let back: WsServerMsg = serde_json::from_value(json).unwrap();
        assert_eq!(back, msg);
    }

    #[test]
    fn role_is_serialized_in_lowercase() {
        assert_eq!(serde_json::to_string(&RoleDto::Admin).unwrap(), "\"admin\"");
        assert_eq!(
            serde_json::from_str::<RoleDto>("\"user\"").unwrap(),
            RoleDto::User
        );
    }

    #[test]
    fn user_dto_field_names_are_the_contract() {
        let u = UserDto {
            id: 1,
            username: "bob".into(),
            display_name: "Bob".into(),
            email: String::new(),
            mobile: String::new(),
            role: RoleDto::User,
            is_active: true,
            last_login_at: None,
        };
        let v = serde_json::to_value(&u).unwrap();
        for key in [
            "id",
            "username",
            "display_name",
            "email",
            "mobile",
            "role",
            "is_active",
            "last_login_at",
        ] {
            assert!(v.get(key).is_some(), "{key}");
        }
        assert!(v.get("password").is_none() && v.get("password_hash").is_none());
    }

    #[test]
    fn setting_update_never_appears_in_debug_and_setting_dto_has_no_raw_value_field() {
        let u = SettingUpdate {
            value: "cle-secrete".into(),
        };
        assert!(!format!("{u:?}").contains("cle-secrete"));
        let d = SettingDto {
            key: "SMS_FREE_KEY".into(),
            value_type: "str".into(),
            category: "Notifications".into(),
            description: String::new(),
            is_secret: true,
            value: "********".into(),
            is_set: true,
            from_default: false,
            readable: true,
            updated_at: None,
        };
        let v = serde_json::to_value(&d).unwrap();
        for key in [
            "key",
            "value_type",
            "category",
            "description",
            "is_secret",
            "value",
            "is_set",
            "from_default",
            "readable",
            "updated_at",
        ] {
            assert!(v.get(key).is_some(), "{key}");
        }
    }

    #[test]
    fn passwords_never_appear_in_debug() {
        let l = LoginRequest {
            username: "bob".into(),
            password: "secret-pw".into(),
        };
        let c = ChangePasswordRequest {
            current_password: "old-pw".into(),
            new_password: "new-pw".into(),
        };
        let dump = format!("{l:?} {c:?}");
        for s in ["secret-pw", "old-pw", "new-pw"] {
            assert!(!dump.contains(s), "{s}");
        }
        assert!(dump.contains("bob"));
    }

    #[test]
    fn websocket_messages_have_a_stable_wire_format() {
        let hello = serde_json::to_string(&WsServerMsg::Hello {
            username: "bob".into(),
        })
        .unwrap();
        assert_eq!(hello, r#"{"type":"hello","username":"bob"}"#);
        assert_eq!(
            serde_json::to_string(&WsServerMsg::Ping).unwrap(),
            r#"{"type":"ping"}"#
        );
        let n = WsServerMsg::Notification {
            level: NotificationLevel::Warning,
            text: "x".into(),
        };
        assert_eq!(
            serde_json::to_string(&n).unwrap(),
            r#"{"type":"notification","level":"warning","text":"x"}"#
        );
        assert_eq!(
            serde_json::from_str::<WsClientMsg>(r#"{"type":"pong"}"#).unwrap(),
            WsClientMsg::Pong
        );
        assert!(serde_json::from_str::<WsClientMsg>(r#"{"type":"nope"}"#).is_err());
    }

    #[test]
    fn android_events_have_a_stable_json_shape() {
        let msg = WsServerMsg::Android {
            device_id: 3,
            event: AndroidEvent::SmsSentAck {
                message_id: 9,
                success: false,
                error: "no signal".into(),
            },
        };
        let json: serde_json::Value = serde_json::to_value(&msg).unwrap();
        assert_eq!(
            json,
            serde_json::json!({"type": "android", "device_id": 3, "event": {"kind": "sms_sent_ack", "message_id": 9, "success": false, "error": "no signal"}})
        );
        assert_eq!(serde_json::from_value::<WsServerMsg>(json).unwrap(), msg);
        let status =
            serde_json::to_string(&AndroidEvent::DeviceStatus { is_online: true }).unwrap();
        assert_eq!(status, r#"{"kind":"device_status","is_online":true}"#);
    }

    #[test]
    fn topics_round_trip_and_only_the_canonical_form_is_accepted() {
        let uuid = uuid::Uuid::parse_str("123e4567-e89b-12d3-a456-426614174000").unwrap();
        for (text, topic) in [
            ("notifications", Topic::Notifications),
            ("system", Topic::System),
            ("alerts", Topic::Alerts),
            ("device:7", Topic::Device(7)),
            ("device:9223372036854775807", Topic::Device(i64::MAX)),
            ("android:3", Topic::Android(3)),
            ("message:12", Topic::Message(12)),
            ("discovery", Topic::Discovery),
            (
                "camera:123e4567-e89b-12d3-a456-426614174000",
                Topic::Camera(uuid),
            ),
        ] {
            assert_eq!(text.parse::<Topic>(), Ok(topic.clone()), "{text}");
            assert_eq!(topic.to_string(), text);
        }
        for bad in [
            "",
            " system",
            "system ",
            "System",
            "device",
            "device:",
            "device:0",
            "device:-3",
            "device:07",
            "device:+7",
            "device:7:8",
            "device:abc",
            "android:0",
            "message:0",
            "message:012",
            "Message:1",
            "android:03",
            "android:-1",
            "android:",
            "Android:3",
            "device: 7",
            "camera:xyz",
            "camera:123E4567-E89B-12D3-A456-426614174000",
            "camera:123e4567e89b12d3a456426614174000",
            "devices:7",
            "notifications:1",
            "all",
            "*",
            "#",
        ] {
            assert_eq!(bad.parse::<Topic>(), Err(InvalidTopic), "{bad:?}");
        }
    }

    #[test]
    fn the_discovery_view_follows_the_events() {
        let device = |ip: &str| DiscoveredDto {
            ip: ip.into(),
            port: 80,
            name: String::new(),
            hardware: String::new(),
            known: false,
        };
        let mut v = DiscoveryView::default();
        v.apply(DiscoveryEvent::Scanning);
        v.apply(DiscoveryEvent::Found {
            device: device("10.0.0.2"),
        });
        v.apply(DiscoveryEvent::Found {
            device: device("10.0.0.2"),
        });
        v.apply(DiscoveryEvent::Found {
            device: device("10.0.0.3"),
        });
        assert!(v.scanning && !v.done);
        assert_eq!(v.found.len(), 2, "une adresse n'est listee qu'une fois");
        v.apply(DiscoveryEvent::Done);
        assert!(!v.scanning && v.done);
        // A new search restarts from zero.
        v.apply(DiscoveryEvent::Scanning);
        assert!(v.found.is_empty() && !v.done && v.scanning);
        v.apply(DiscoveryEvent::Failed {
            message: "x".into(),
        });
        assert_eq!((v.scanning, v.error.as_deref()), (false, Some("x")));
    }

    #[test]
    fn the_new_messages_have_the_wire_format_of_the_reference_version() {
        let state = WsServerMsg::DeviceState(DeviceStateDto {
            device_id: 42,
            friendly_name: "Bureau".into(),
            state: serde_json::json!({"temperature": 22.5, "humidity": 48}),
            available: true,
        });
        // Contract of appendix A.1: flat fields next to `type`.
        assert_eq!(
            serde_json::to_value(&state).unwrap(),
            serde_json::json!({"type": "device_state", "device_id": 42, "friendly_name": "Bureau",
                               "state": {"temperature": 22.5, "humidity": 48}, "available": true})
        );
        assert_eq!(
            serde_json::from_str::<WsServerMsg>(&serde_json::to_string(&state).unwrap()).unwrap(),
            state
        );
        assert_eq!(
            serde_json::to_string(&WsServerMsg::Subscribed {
                topic: "device:7".into()
            })
            .unwrap(),
            r#"{"type":"subscribed","topic":"device:7"}"#
        );
        assert_eq!(
            serde_json::to_string(&WsServerMsg::Refused {
                topic: "x".into(),
                reason: RefusalReason::Forbidden
            })
            .unwrap(),
            r#"{"type":"refused","topic":"x","reason":"forbidden"}"#
        );
        assert_eq!(
            serde_json::from_str::<WsClientMsg>(r#"{"type":"subscribe","topic":"device:7"}"#)
                .unwrap(),
            WsClientMsg::Subscribe {
                topic: "device:7".into()
            }
        );
        assert_eq!(
            serde_json::from_str::<WsClientMsg>(r#"{"type":"unsubscribe","topic":"system"}"#)
                .unwrap(),
            WsClientMsg::Unsubscribe {
                topic: "system".into()
            }
        );
        assert!(
            serde_json::from_str::<WsClientMsg>(r#"{"type":"subscribe"}"#).is_err(),
            "sujet obligatoire"
        );
    }
}
