// SPDX-License-Identifier: AGPL-3.0-or-later
//! Typed configuration, read from the environment.
//!
//! The variable names are those of DomiNet's `.env.example.prod` when they make sense. All the
//! errors are collected then reported together. Secrets are wrapped in [`Secret`] so that they never
//! appear in a `Debug` nor in the logs.

use std::fmt;
use std::net::SocketAddr;

/// Sensitive value: `Debug` never shows the content.
#[derive(Clone, PartialEq, Eq)]
pub struct Secret(String);

impl Secret {
    #[must_use]
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    /// Explicit access to the content (to be used only to use it, never to display it).
    #[must_use]
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Secret(***)")
    }
}

/// A user name and a password that DomiNet itself presents to another service.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServiceCredential {
    pub user: String,
    pub password: Secret,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogFormat {
    Pretty,
    Json,
}

#[derive(Debug, Clone)]
pub struct MqttConfig {
    pub host: String,
    pub port: u16,
    pub user: Option<String>,
    pub password: Option<Secret>,
    /// `MQTT_CLIENT_ID` (default `dominet`): stable client identifier. Different from that of the reference implementation
    /// (`dominet-automation`) so that both can coexist on the same broker during the migration.
    pub client_id: String,
}

#[derive(Debug, Clone)]
pub struct ReductConfig {
    pub url: String,
    pub token: Option<Secret>,
    /// `IOT_QUOTA_GB` (default 5): FIFO quota of the `iot` bucket, applied at its creation.
    pub iot_quota_gb: u32,
    /// `REDUCTSTORE_IOT_BUCKET` (default `iot`): changes the bucket name (tests and trials on a shared stack).
    pub iot_bucket: String,
    /// `VIDEO_QUOTA_GB` (default 50, like the reference): FIFO quota of the video segments bucket, applied at its creation.
    pub video_quota_gb: u32,
    /// `REDUCTSTORE_VIDEO_BUCKET` (default `surveillance`, like the reference).
    pub video_bucket: String,
}

/// SolarMax inverter to query (`SOLARMAX_*`). Without `SOLARMAX_HOST`, no publisher: the address has **no default value**,
/// so that a trial never contacts the production inverter by mistake.
#[derive(Debug, Clone)]
pub struct SolarMaxConfig {
    pub host: String,
    /// `SOLARMAX_PORT` (default 12345).
    pub port: u16,
    /// `SOLARMAX_INVERTER_ID` (default 1).
    pub inverter: u8,
    /// `SOLARMAX_TOPIC` (default that of the reference implementation).
    pub topic: String,
    /// `SOLARMAX_INTERVAL_SECS` (default 10).
    pub interval_secs: u32,
}

/// Production topic of the reference implementation (its devices and dashboards refer to it).
pub const DEFAULT_SOLARMAX_TOPIC: &str = "solarmax/0x6f0930a64c8b8803/prod";

/// SMTP server for e-mails (`EMAIL_*`, same names as the reference implementation). Absent as long as `EMAIL_HOST` is not set.
#[derive(Debug, Clone)]
pub struct EmailConfig {
    pub host: String,
    /// `EMAIL_PORT` (default 587, or 465 with `EMAIL_USE_SSL`).
    pub port: u16,
    pub user: Option<String>,
    pub password: Option<Secret>,
    pub security: dominet_notify::Security,
    /// `EMAIL_TIMEOUT_SECS` (default 10).
    pub timeout_secs: u32,
}

#[derive(Debug, Clone)]
pub struct Config {
    /// `HTTP_ADDR` (default `0.0.0.0:8000`).
    pub http_addr: SocketAddr,
    /// `METRICS_ADDR` (default `0.0.0.0:9100`, `off` to disable): `/metrics` listener for Prometheus, never published on the host (ADR 0051).
    pub metrics_addr: Option<SocketAddr>,
    /// `DATABASE_URL` (required, contains a password).
    pub database_url: Secret,
    /// `DOMINET_MASTER_KEY`: AES-256 key (32 bytes in base64) for encrypting secrets at rest.
    pub master_key: Option<Secret>,
    /// `SESSION_IDLE_MINUTES` (default 480): logout after this long without activity.
    pub session_idle_minutes: u32,
    /// `SESSION_MAX_DAYS` (default 14): maximum lifetime of a session, active or not.
    pub session_max_days: u32,
    /// `WS_HEARTBEAT_SECS` (default 25): WebSocket heartbeat interval; at each heartbeat the session is revalidated.
    pub ws_heartbeat_secs: u32,
    /// `PTZ_DEAD_MAN_SECS` (default 8): a continuous PTZ movement that is not refreshed is stopped after this delay.
    pub ptz_dead_man_secs: u32,
    /// `SYSTEM_STATS_SECS` (default 5): measurement interval of the machine's resources (sidebar gauges).
    pub system_stats_secs: u32,
    /// `TRUSTED_PROXIES`: addresses or CIDR networks of the proxies whose `X-Forwarded-For` is trusted.
    pub trusted_proxies: Vec<ipnet::IpNet>,
    /// `SITE_ROOT` (default `target/site`): folder of the interface's static files (JS/WASM/CSS).
    pub site_root: String,
    /// `DOCS_ROOT` (default `target/docs-site`): MkDocs documentation site, served on `/docs/` behind the login.
    pub docs_root: String,
    /// `CUSTOM_CSS`: appearance customisation file, served on `/custom.css` after the main stylesheet (re-read at each
    /// request: a change shows on reload, without rebuilding). Absent or not found: empty stylesheet.
    pub custom_css: Option<String>,
    pub allowed_hosts: Vec<String>,
    pub csrf_trusted_origins: Vec<String>,
    /// `COOKIE_SECURE` (default `true`): set to `false` over HTTP on the LAN.
    pub cookie_secure: bool,
    /// `MCP_ENABLED` (default `false`): MCP server on `/mcp` (R11, ADR 0058). Without an access token created in the administration
    /// (ADR 0059), nobody gets in.
    pub mcp_enabled: bool,
    /// `MCP_RATE_LIMIT_PER_MIN` (default 120): calls per minute and per token; beyond that, 429.
    pub mcp_rate_limit_per_min: u32,
    /// `MCP_AUDIT_RETENTION_DAYS` (default 365): maximum age of the MCP audit log lines, purged daily.
    pub mcp_audit_retention_days: u32,
    /// `TZ` (default `Europe/Paris`).
    pub timezone: String,
    /// `LOG_FORMAT` = `pretty` (default) or `json`. The filter is set by `RUST_LOG`.
    pub log_format: LogFormat,
    /// `MQTT_HOST` (+ `MQTT_PORT`, `MQTT_USER`, `MQTT_PASSWORD`): absent as long as MQTT is not configured.
    pub mqtt: Option<MqttConfig>,
    /// `REDUCTSTORE_URL` (+ `REDUCTSTORE_TOKEN`, `IOT_QUOTA_GB`).
    pub reductstore: Option<ReductConfig>,
    /// `OPEN_METEO_URL` (default: Open-Meteo's public API): the tests and the CI talk to a simulated API.
    pub open_meteo_url: String,
    /// `SOLARMAX_HOST` (+ `SOLARMAX_PORT`, `SOLARMAX_INVERTER_ID`, `SOLARMAX_TOPIC`, `SOLARMAX_INTERVAL_SECS`).
    pub solarmax: Option<SolarMaxConfig>,
    /// `EMAIL_HOST` (+ `EMAIL_PORT`, `EMAIL_HOST_USER`, `EMAIL_HOST_PASSWORD`, `EMAIL_USE_TLS`, `EMAIL_USE_SSL`, `EMAIL_TIMEOUT_SECS`).
    pub email: Option<EmailConfig>,
    /// `SMS_FREE_URL` (default: Free Mobile's public API): the tests and the demonstration talk to a fake server.
    pub sms_free_url: String,
    /// `GRAFANA_DASHBOARDS_DIR`: folder watched by Grafana, where the device dashboards are written as JSON files.
    pub grafana_dashboards_dir: Option<String>,
    /// `BACKUP_DIR` (e.g. `/backups`): backups folder; absent, the scheduled backup is disabled and `dominet backup` refuses.
    pub backup_dir: Option<String>,
    /// `BACKUP_RETENTION_DAYS` (default 7, like the reference): maximum age of the backups kept (the most recent is always kept).
    pub backup_retention_days: u32,
    /// `BACKUP_HOUR` (default 3, local time): hour of the daily backup.
    pub backup_hour: u32,
    /// `REDUCTSTORE_BACKUP_SRC` (e.g. `/reductstore-src`, read-only volume): ReductStore data to archive; absent, only the database is.
    pub reductstore_backup_src: Option<String>,
    /// `PG_DUMP_BIN` / `PSQL_BIN` (default `pg_dump` / `psql`): PostgreSQL clients, tests and special installations.
    pub pg_dump_bin: String,
    pub psql_bin: String,
    /// `PROMETHEUS_RULES_PATH`: `rules.yml` file that Prometheus re-reads; absent, the rules are not written.
    pub prometheus_rules_path: Option<String>,
    /// `PROMETHEUS_URL` (e.g. `http://prometheus:9090`): `POST /-/reload` after each write.
    pub prometheus_url: Option<String>,
    /// `PROMETHEUS_ALERTS_POLL_SECS` (default 60, like the reference): polling interval of `/api/v1/alerts`.
    pub prometheus_alerts_poll_secs: u32,
    /// `PROMETHEUS_ALERT_COOLDOWN` (seconds, default 3600): silence of an already notified alert.
    pub prometheus_alert_cooldown_secs: u32,
    /// `GRAFANA_ALERTING_PATH`: `rules.yaml` file of Grafana's alerting provisioning.
    pub grafana_alerting_path: Option<String>,
    /// `GRAFANA_URL` (e.g. `http://grafana:3000`): reload of the alerting provisioning.
    pub grafana_url: Option<String>,
    /// `GRAFANA_USER` (default `admin`) and `GRAFANA_PASSWORD`: account used for the reload.
    pub grafana_user: String,
    pub grafana_password: Option<Secret>,
    /// `MEDIAMTX_API_URL` (e.g. `http://mediamtx:9997`): MediaMTX REST API; absent, the cameras are not synchronised.
    pub mediamtx_api_url: Option<String>,
    /// `MEDIAMTX_RTSP_URL` (e.g. `rtsp://mediamtx:8554`): re-stream that the `video` role reads (`<url>/<uuid>`); without it, no images.
    pub mediamtx_rtsp_url: Option<String>,
    /// `MEDIAMTX_READ_USER` and `MEDIAMTX_READ_PASSWORD` (both or none): the service credential that DomiNet's own reads of the
    /// re-stream (`video` role) present to MediaMTX, which asks DomiNet to authorise every read (ADR 0064).
    pub mediamtx_read: Option<ServiceCredential>,
    /// `SNAPSHOTS_DIR` (default `data/snapshots`): indexed snapshots and, under `latest/`, each camera's last image.
    pub snapshots_dir: String,
    /// `SNAPSHOTS_MAX_PER_CAMERA` (default 500): beyond that, a camera's oldest indexed snapshots are deleted.
    pub snapshots_max_per_camera: u32,
    /// `RECORDINGS_DIR` (default `/recordings`, like the reference): folder where MediaMTX writes the segments (`<folder>/<uuid>/<date>.mp4`).
    pub recordings_dir: String,
    /// `RECORDINGS_SEGMENT_DURATION` (default 60 s): duration of a segment, the same as MediaMTX's `recordSegmentDuration`.
    pub recordings_segment_secs: u32,
    /// `RECORDINGS_SCAN_SECS` (default 60): scan interval for finished segments.
    pub recordings_scan_secs: u32,
    /// `RECORDINGS_GRACE_SECS` (default 15): a segment not modified for this long is considered finished.
    pub recordings_grace_secs: u32,
    /// `MOTION_THRESHOLD` (default 0.01, like the reference): share of the image that must move (0 to 1) to declare a motion.
    pub motion_threshold: f64,
    /// `MOTION_INTERVAL_SECS` (default 30, like the reference's task): interval of motion evaluation per camera in `motion` mode.
    pub motion_interval_secs: u32,
    /// `MOTION_ALERT_COOLDOWN` (default 600 s) and `DETECTION_ALERT_COOLDOWN` (default 300 s): minimum delay between two notifications from a camera.
    pub motion_alert_cooldown_secs: u32,
    pub detection_alert_cooldown_secs: u32,
    /// `YOLO_MODEL_PATH` (default `models/yolov8n.onnx`, `make fetch-model`): model used when the detector setting gives none.
    pub yolo_model_path: String,
    /// `YOLO_WORKERS` (default 1): ONNX Runtime sessions with one compute thread each.
    pub yolo_workers: u32,
    /// `DETECTIONS_DIR` (default `data/detections`): annotated images of the detections (the detector's "save the images" setting).
    pub detections_dir: String,
    /// `FFMPEG_HWACCEL`: `auto` (default, VAAPI if available) or `off` (software decoding).
    pub ffmpeg_hwaccel_auto: bool,
    /// `HISTORY_RETENTION_DAYS` (default 400): maximum age of the `device_state_log` rows, purged daily.
    pub history_retention_days: u32,
    /// `ENERGY_LINKY_DEVICE` / `ENERGY_SOLAR_DEVICE` / `ENERGY_PAC_DEVICE` (defaults `consommation`, `prod`, `PAC-CE`): devices of the
    /// daily energy summary.
    pub energy: dominet_automation::energy::EnergyDevices,
}

impl EmailConfig {
    /// Parameters of the sending crate (the password is only copied there to be used).
    #[must_use]
    pub fn smtp(&self) -> dominet_notify::SmtpConfig {
        dominet_notify::SmtpConfig {
            host: self.host.clone(),
            port: self.port,
            user: self.user.clone(),
            password: self.password.as_ref().map(|p| p.expose().to_string()),
            security: self.security,
            timeout: std::time::Duration::from_secs(u64::from(self.timeout_secs)),
        }
    }
}

/// All the configuration errors, one per line.
#[derive(Debug, thiserror::Error)]
#[error("configuration invalide :\n  - {}", .0.join("\n  - "))]
pub struct ConfigError(pub Vec<String>);

/// Environment reader: collects the errors instead of stopping at the first one.
struct Env<'a> {
    get: &'a dyn Fn(&str) -> Option<String>,
    errors: Vec<String>,
}

impl Env<'_> {
    /// Cleaned value; an empty value counts as absent.
    fn var(&self, key: &str) -> Option<String> {
        (self.get)(key)
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty())
    }

    fn fail(&mut self, msg: String) {
        self.errors.push(msg);
    }

    fn list(&self, key: &str) -> Vec<String> {
        self.var(key)
            .map(|v| {
                v.split(',')
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect()
            })
            .unwrap_or_default()
    }

    fn http_addr(&mut self) -> SocketAddr {
        let default = SocketAddr::from(([0, 0, 0, 0], 8000));
        let Some(v) = self.var("HTTP_ADDR") else {
            return default;
        };
        v.parse().unwrap_or_else(|_| {
            self.fail(format!(
                "HTTP_ADDR: invalid address \"{v}\" (expected host:port, e.g. 0.0.0.0:8000)"
            ));
            default
        })
    }

    fn metrics_addr(&mut self) -> Option<SocketAddr> {
        let default = SocketAddr::from(([0, 0, 0, 0], 9100));
        let Some(v) = self.var("METRICS_ADDR") else {
            return Some(default);
        };
        if v.eq_ignore_ascii_case("off") {
            return None;
        }
        Some(v.parse().unwrap_or_else(|_| {
            self.fail(format!(
                "METRICS_ADDR: invalid address \"{v}\" (expected host:port, or off)"
            ));
            default
        }))
    }

    fn database_url(&mut self) -> Option<Secret> {
        match self.var("DATABASE_URL") {
            None => {
                self.fail(
                    "DATABASE_URL: mandatory (e.g. postgres://user:password@host:5432/database)"
                        .into(),
                );
                None
            }
            Some(v) if v.starts_with("postgres://") || v.starts_with("postgresql://") => {
                Some(Secret::new(v))
            }
            Some(_) => {
                self.fail("DATABASE_URL: must start with postgres:// or postgresql://".into());
                None
            }
        }
    }

    fn mediamtx_read(&mut self) -> Option<ServiceCredential> {
        match (
            self.var("MEDIAMTX_READ_USER"),
            self.var("MEDIAMTX_READ_PASSWORD"),
        ) {
            (Some(user), Some(password)) => Some(ServiceCredential {
                user,
                password: Secret::new(password),
            }),
            (None, None) => None,
            _ => {
                self.fail("MEDIAMTX_READ_USER and MEDIAMTX_READ_PASSWORD: set both or none".into());
                None
            }
        }
    }

    fn master_key(&mut self) -> Option<Secret> {
        let v = self.var("DOMINET_MASTER_KEY")?;
        if crate::secrets::MasterKey::parse(&v).is_err() {
            self.fail("DOMINET_MASTER_KEY: 32 bytes encoded in base64 expected (generate with `dominet gen-master-key`)".into());
            return None;
        }
        Some(Secret::new(v))
    }

    fn cookie_secure(&mut self) -> bool {
        let Some(v) = self.var("COOKIE_SECURE") else {
            return true;
        };
        parse_bool(&v).unwrap_or_else(|| {
            self.fail(format!(
                "COOKIE_SECURE: boolean expected (true/false), got \"{v}\""
            ));
            true
        })
    }

    /// `MCP_ENABLED`: the MCP server is off by default.
    fn mcp_enabled(&mut self) -> bool {
        match self.var("MCP_ENABLED") {
            None => false,
            Some(v) => parse_bool(&v).unwrap_or_else(|| {
                self.fail(format!(
                    "MCP_ENABLED: boolean expected (true/false), got \"{v}\""
                ));
                false
            }),
        }
    }

    fn log_format(&mut self) -> LogFormat {
        match self.var("LOG_FORMAT").as_deref() {
            None | Some("pretty") => LogFormat::Pretty,
            Some("json") => LogFormat::Json,
            Some(other) => {
                self.fail(format!(
                    "LOG_FORMAT: \"pretty\" or \"json\" expected, got \"{other}\""
                ));
                LogFormat::Pretty
            }
        }
    }

    /// `FFMPEG_HWACCEL`: `auto` (default) or `off`.
    /// A ratio between 0 (excluded) and 1 (included).
    fn ratio(&mut self, key: &str, default: f64) -> f64 {
        let Some(v) = self.var(key) else {
            return default;
        };
        match v.parse::<f64>() {
            Ok(n) if n > 0.0 && n <= 1.0 => n,
            _ => {
                self.fail(format!(
                    "{key}: ratio between 0 (excluded) and 1 expected, got \"{v}\""
                ));
                default
            }
        }
    }

    fn hwaccel(&mut self) -> bool {
        match self
            .var("FFMPEG_HWACCEL")
            .as_deref()
            .map(str::to_ascii_lowercase)
            .as_deref()
        {
            None | Some("auto") => true,
            Some("off") => false,
            Some(other) => {
                self.fail(format!(
                    "FFMPEG_HWACCEL: `auto` or `off` expected, not \"{other}\""
                ));
                true
            }
        }
    }

    /// Hour of the day (0 to 23), `default` if absent.
    fn hour(&mut self, key: &str, default: u32) -> u32 {
        let Some(v) = self.var(key) else {
            return default;
        };
        match v.parse::<u32>() {
            Ok(n) if n < 24 => n,
            _ => {
                self.fail(format!(
                    "{key}: hour between 0 and 23 expected, got \"{v}\""
                ));
                default
            }
        }
    }

    fn positive_u32(&mut self, key: &str, default: u32, max: u32) -> u32 {
        let Some(v) = self.var(key) else {
            return default;
        };
        match v.parse::<u32>() {
            Ok(n) if (1..=max).contains(&n) => n,
            _ => {
                self.fail(format!(
                    "{key}: integer between 1 and {max} expected, got \"{v}\""
                ));
                default
            }
        }
    }

    fn trusted_proxies(&mut self) -> Vec<ipnet::IpNet> {
        let mut out = Vec::new();
        for item in self.list("TRUSTED_PROXIES") {
            // A single address is a /32 network (or /128).
            let parsed = item
                .parse::<ipnet::IpNet>()
                .or_else(|_| item.parse::<std::net::IpAddr>().map(ipnet::IpNet::from));
            match parsed {
                Ok(net) => out.push(net),
                Err(_) => self.fail(format!(
                    "TRUSTED_PROXIES: invalid address or CIDR network \"{item}\""
                )),
            }
        }
        out
    }

    fn mqtt(&mut self) -> Option<MqttConfig> {
        let host = self.var("MQTT_HOST")?;
        let port = match self.var("MQTT_PORT") {
            None => 1883,
            Some(p) => p.parse().unwrap_or_else(|_| {
                self.fail(format!("MQTT_PORT: invalid port \"{p}\""));
                1883
            }),
        };
        Some(MqttConfig {
            host,
            port,
            user: self.var("MQTT_USER"),
            password: self.var("MQTT_PASSWORD").map(Secret::new),
            client_id: self
                .var("MQTT_CLIENT_ID")
                .unwrap_or_else(|| "dominet".to_string()),
        })
    }

    fn email(&mut self) -> Option<EmailConfig> {
        use dominet_notify::Security;
        let host = self.var("EMAIL_HOST")?;
        let flag = |this: &mut Self, key: &str, default: bool| match this.var(key) {
            None => default,
            Some(v) => parse_bool(&v).unwrap_or_else(|| {
                this.fail(format!("{key}: boolean expected (true/false), got \"{v}\""));
                default
            }),
        };
        let tls = flag(self, "EMAIL_USE_TLS", true);
        let ssl = flag(self, "EMAIL_USE_SSL", false);
        if tls && ssl && self.var("EMAIL_USE_TLS").is_some() {
            self.fail("EMAIL_USE_TLS and EMAIL_USE_SSL are mutually exclusive".into());
        }
        // SSL takes precedence over the TLS default; with neither (explicitly), no encryption (loopback only).
        let security = if ssl {
            Security::Tls
        } else if tls {
            Security::StartTls
        } else {
            Security::Plain
        };
        let default_port = if security == Security::Tls { 465 } else { 587 };
        let port = match self.var("EMAIL_PORT") {
            None => default_port,
            Some(p) => p.parse().unwrap_or_else(|_| {
                self.fail(format!("EMAIL_PORT: invalid port \"{p}\""));
                default_port
            }),
        };
        let user = self.var("EMAIL_HOST_USER");
        let password = self.var("EMAIL_HOST_PASSWORD").map(Secret::new);
        let timeout_secs = self.positive_u32("EMAIL_TIMEOUT_SECS", 10, 300);
        let cfg = EmailConfig {
            host,
            port,
            user,
            password,
            security,
            timeout_secs,
        };
        if let Err(e) = cfg.smtp().validate() {
            self.fail(format!("EMAIL_* : {e}"));
        }
        Some(cfg)
    }

    fn solarmax(&mut self) -> Option<SolarMaxConfig> {
        let host = self.var("SOLARMAX_HOST")?;
        let port = match self.var("SOLARMAX_PORT") {
            None => 12345,
            Some(p) => p.parse().unwrap_or_else(|_| {
                self.fail(format!("SOLARMAX_PORT: invalid port \"{p}\""));
                12345
            }),
        };
        let inverter = match self.var("SOLARMAX_INVERTER_ID") {
            None => 1,
            Some(n) => n
                .parse::<u8>()
                .ok()
                .filter(|n| (1..=99).contains(n))
                .unwrap_or_else(|| {
                    self.fail(format!(
                        "SOLARMAX_INVERTER_ID: integer between 1 and 99 expected, got \"{n}\""
                    ));
                    1
                }),
        };
        let topic = self
            .var("SOLARMAX_TOPIC")
            .unwrap_or_else(|| DEFAULT_SOLARMAX_TOPIC.to_string());
        if !dominet_domain::device::is_valid_friendly_name(&topic) {
            self.fail(format!(
                "SOLARMAX_TOPIC: invalid publication topic \"{topic}\""
            ));
        }
        let interval_secs = self.positive_u32("SOLARMAX_INTERVAL_SECS", 10, 3600);
        Some(SolarMaxConfig {
            host,
            port,
            inverter,
            topic,
            interval_secs,
        })
    }

    fn reductstore(&mut self) -> Option<ReductConfig> {
        let url = self.var("REDUCTSTORE_URL")?;
        if !(url.starts_with("http://") || url.starts_with("https://")) {
            self.fail("REDUCTSTORE_URL: must start with http:// or https://".into());
        }
        let iot_quota_gb = self.positive_u32("IOT_QUOTA_GB", 5, 100_000);
        let video_quota_gb = self.positive_u32("VIDEO_QUOTA_GB", 50, 100_000);
        Some(ReductConfig {
            url,
            token: self.var("REDUCTSTORE_TOKEN").map(Secret::new),
            iot_quota_gb,
            iot_bucket: self
                .var("REDUCTSTORE_IOT_BUCKET")
                .unwrap_or_else(|| "iot".to_string()),
            video_quota_gb,
            video_bucket: self
                .var("REDUCTSTORE_VIDEO_BUCKET")
                .unwrap_or_else(|| "surveillance".to_string()),
        })
    }
}

impl Config {
    /// Reads the configuration from the process environment.
    ///
    /// # Errors
    /// Returns all the missing or invalid variables.
    pub fn from_env() -> Result<Self, ConfigError> {
        Self::from_lookup(|key| std::env::var(key).ok())
    }

    /// Reads the configuration through a lookup function (testable without touching the environment).
    ///
    /// # Errors
    /// Returns all the missing or invalid variables.
    #[allow(
        clippy::too_many_lines,
        reason = "one line per environment variable: wrapping would not help reading"
    )]
    pub fn from_lookup(get: impl Fn(&str) -> Option<String>) -> Result<Self, ConfigError> {
        let mut env = Env {
            get: &get,
            errors: Vec::new(),
        };
        let http_addr = env.http_addr();
        let metrics_addr = env.metrics_addr();
        let database_url = env.database_url();
        let master_key = env.master_key();
        let cookie_secure = env.cookie_secure();
        let mcp_enabled = env.mcp_enabled();
        let mcp_rate_limit_per_min = env.positive_u32("MCP_RATE_LIMIT_PER_MIN", 120, 6_000);
        let mcp_audit_retention_days = env.positive_u32("MCP_AUDIT_RETENTION_DAYS", 365, 36_500);
        let log_format = env.log_format();
        let session_idle_minutes = env.positive_u32("SESSION_IDLE_MINUTES", 480, 60 * 24 * 30);
        let session_max_days = env.positive_u32("SESSION_MAX_DAYS", 14, 365);
        let ws_heartbeat_secs = env.positive_u32("WS_HEARTBEAT_SECS", 25, 300);
        let ptz_dead_man_secs = env.positive_u32("PTZ_DEAD_MAN_SECS", 8, 120);
        let snapshots_max_per_camera = env.positive_u32("SNAPSHOTS_MAX_PER_CAMERA", 500, 100_000);
        let recordings_segment_secs = env.positive_u32("RECORDINGS_SEGMENT_DURATION", 60, 3600);
        let recordings_scan_secs = env.positive_u32("RECORDINGS_SCAN_SECS", 60, 3600);
        let recordings_grace_secs = env.positive_u32("RECORDINGS_GRACE_SECS", 15, 3600);
        let motion_threshold = env.ratio("MOTION_THRESHOLD", 0.01);
        let motion_interval_secs = env.positive_u32("MOTION_INTERVAL_SECS", 30, 3600);
        let motion_alert_cooldown_secs = env.positive_u32("MOTION_ALERT_COOLDOWN", 600, 86_400);
        let detection_alert_cooldown_secs =
            env.positive_u32("DETECTION_ALERT_COOLDOWN", 300, 86_400);
        let yolo_workers = env.positive_u32("YOLO_WORKERS", 1, 16);
        let ffmpeg_hwaccel_auto = env.hwaccel();
        let system_stats_secs = env.positive_u32("SYSTEM_STATS_SECS", 5, 3600);
        let trusted_proxies = env.trusted_proxies();
        let mqtt = env.mqtt();
        let reductstore = env.reductstore();
        let mediamtx_read = env.mediamtx_read();
        let solarmax = env.solarmax();
        let email = env.email();
        let history_retention_days = env.positive_u32("HISTORY_RETENTION_DAYS", 400, 36_500);

        let Some(database_url) = database_url else {
            return Err(ConfigError(env.errors));
        };
        if !env.errors.is_empty() {
            return Err(ConfigError(env.errors));
        }
        Ok(Self {
            http_addr,
            metrics_addr,
            database_url,
            master_key,
            session_idle_minutes,
            session_max_days,
            ws_heartbeat_secs,
            ptz_dead_man_secs,
            system_stats_secs,
            trusted_proxies,
            site_root: env.var("SITE_ROOT").unwrap_or_else(|| "target/site".into()),
            docs_root: env
                .var("DOCS_ROOT")
                .unwrap_or_else(|| "target/docs-site".into()),
            custom_css: env.var("CUSTOM_CSS"),
            allowed_hosts: env.list("ALLOWED_HOSTS"),
            csrf_trusted_origins: env.list("CSRF_TRUSTED_ORIGINS"),
            cookie_secure,
            mcp_enabled,
            mcp_rate_limit_per_min,
            mcp_audit_retention_days,
            timezone: env.var("TZ").unwrap_or_else(|| "Europe/Paris".into()),
            log_format,
            mqtt,
            reductstore,
            history_retention_days,
            energy: {
                let d = dominet_automation::energy::EnergyDevices::default();
                dominet_automation::energy::EnergyDevices {
                    linky: env.var("ENERGY_LINKY_DEVICE").unwrap_or(d.linky),
                    solar: env.var("ENERGY_SOLAR_DEVICE").unwrap_or(d.solar),
                    pac: env.var("ENERGY_PAC_DEVICE").unwrap_or(d.pac),
                }
            },
            open_meteo_url: env
                .var("OPEN_METEO_URL")
                .unwrap_or_else(|| dominet_integrations::weather::DEFAULT_URL.to_string()),
            solarmax,
            email,
            sms_free_url: env
                .var("SMS_FREE_URL")
                .unwrap_or_else(|| dominet_notify::DEFAULT_FREE_MOBILE_URL.to_string()),
            grafana_dashboards_dir: env.var("GRAFANA_DASHBOARDS_DIR"),
            backup_dir: env.var("BACKUP_DIR"),
            backup_retention_days: env.positive_u32("BACKUP_RETENTION_DAYS", 7, 3650),
            backup_hour: env.hour("BACKUP_HOUR", 3),
            reductstore_backup_src: env.var("REDUCTSTORE_BACKUP_SRC"),
            pg_dump_bin: env.var("PG_DUMP_BIN").unwrap_or_else(|| "pg_dump".into()),
            psql_bin: env.var("PSQL_BIN").unwrap_or_else(|| "psql".into()),
            prometheus_rules_path: env.var("PROMETHEUS_RULES_PATH"),
            prometheus_url: env.var("PROMETHEUS_URL"),
            prometheus_alerts_poll_secs: env.positive_u32(
                "PROMETHEUS_ALERTS_POLL_SECS",
                60,
                86_400,
            ),
            prometheus_alert_cooldown_secs: env.positive_u32(
                "PROMETHEUS_ALERT_COOLDOWN",
                3600,
                604_800,
            ),
            grafana_alerting_path: env.var("GRAFANA_ALERTING_PATH"),
            grafana_url: env.var("GRAFANA_URL"),
            grafana_user: env.var("GRAFANA_USER").unwrap_or_else(|| "admin".into()),
            grafana_password: env.var("GRAFANA_PASSWORD").map(Secret::new),
            mediamtx_api_url: env.var("MEDIAMTX_API_URL"),
            mediamtx_rtsp_url: env.var("MEDIAMTX_RTSP_URL"),
            mediamtx_read,
            snapshots_dir: env
                .var("SNAPSHOTS_DIR")
                .unwrap_or_else(|| "data/snapshots".to_string()),
            snapshots_max_per_camera,
            recordings_dir: env
                .var("RECORDINGS_DIR")
                .unwrap_or_else(|| "/recordings".to_string()),
            recordings_segment_secs,
            recordings_scan_secs,
            recordings_grace_secs,
            motion_threshold,
            motion_interval_secs,
            motion_alert_cooldown_secs,
            detection_alert_cooldown_secs,
            yolo_model_path: env
                .var("YOLO_MODEL_PATH")
                .unwrap_or_else(|| "models/yolov8n.onnx".to_string()),
            yolo_workers,
            detections_dir: env
                .var("DETECTIONS_DIR")
                .unwrap_or_else(|| "data/detections".to_string()),
            ffmpeg_hwaccel_auto,
        })
    }
}

fn parse_bool(v: &str) -> Option<bool> {
    match v.to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "oui" => Some(true),
        "0" | "false" | "no" | "non" => Some(false),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;

    fn load(pairs: &[(&str, &str)]) -> Result<Config, ConfigError> {
        let map: HashMap<String, String> = pairs
            .iter()
            .map(|(k, v)| ((*k).into(), (*v).into()))
            .collect();
        Config::from_lookup(|k| map.get(k).cloned())
    }

    const DB: (&str, &str) = ("DATABASE_URL", "postgres://u:p@localhost/db");

    #[test]
    fn defaults_with_only_database_url() {
        let c = load(&[DB]).unwrap();
        assert_eq!(c.http_addr, SocketAddr::from(([0, 0, 0, 0], 8000)));
        assert!(c.cookie_secure);
        assert_eq!(c.timezone, "Europe/Paris");
        assert_eq!(c.log_format, LogFormat::Pretty);
        assert!(c.mqtt.is_none() && c.reductstore.is_none());
        assert_eq!(c.allowed_hosts.len(), 0);
        assert_eq!(c.history_retention_days, 400);
        // No inverter address by default: without SOLARMAX_HOST, no SolarMax publisher.
        assert!(c.solarmax.is_none());
        assert_eq!(c.open_meteo_url, "https://api.open-meteo.com/v1/forecast");
    }

    #[test]
    fn the_mcp_server_is_off_by_default_with_a_rate_limit_and_an_audit_retention() {
        let c = load(&[DB]).unwrap();
        assert!(!c.mcp_enabled);
        assert_eq!(
            (c.mcp_rate_limit_per_min, c.mcp_audit_retention_days),
            (120, 365)
        );
        let c = load(&[
            DB,
            ("MCP_ENABLED", "oui"),
            ("MCP_RATE_LIMIT_PER_MIN", "30"),
            ("MCP_AUDIT_RETENTION_DAYS", "90"),
        ])
        .unwrap();
        assert!(c.mcp_enabled);
        assert_eq!(
            (c.mcp_rate_limit_per_min, c.mcp_audit_retention_days),
            (30, 90)
        );
        // Invalid values are refused at startup, not ignored.
        assert!(load(&[DB, ("MCP_ENABLED", "peut-etre")]).is_err());
        assert!(load(&[DB, ("MCP_RATE_LIMIT_PER_MIN", "0")]).is_err());
        assert!(load(&[DB, ("MCP_RATE_LIMIT_PER_MIN", "100000")]).is_err());
    }

    #[test]
    fn email_follows_the_reference_variables_and_never_downgrades_silently() {
        use dominet_notify::Security;
        // Without EMAIL_HOST: no e-mail; the Free Mobile API is the official one.
        let c = load(&[DB]).unwrap();
        assert!(c.email.is_none());
        assert_eq!(c.sms_free_url, "https://smsapi.free-mobile.fr/sendmsg");
        // Default: STARTTLS on 587.
        let e = load(&[
            DB,
            ("EMAIL_HOST", "smtp.example.com"),
            ("EMAIL_HOST_USER", "u"),
            ("EMAIL_HOST_PASSWORD", "p"),
        ])
        .unwrap()
        .email
        .unwrap();
        assert_eq!(
            (e.port, e.security, e.timeout_secs),
            (587, Security::StartTls, 10)
        );
        assert!(
            !format!("{e:?}").contains("\"p\""),
            "mot de passe masque : {e:?}"
        );
        // SSL: direct TLS, port 465.
        let e = load(&[DB, ("EMAIL_HOST", "h"), ("EMAIL_USE_SSL", "True")])
            .unwrap()
            .email
            .unwrap();
        assert_eq!((e.port, e.security), (465, Security::Tls));
        // Explicitly without encryption: allowed without credentials, or towards the loopback.
        assert!(load(&[DB, ("EMAIL_HOST", "relais"), ("EMAIL_USE_TLS", "False")]).is_ok());
        assert!(
            load(&[
                DB,
                ("EMAIL_HOST", "127.0.0.1"),
                ("EMAIL_USE_TLS", "no"),
                ("EMAIL_HOST_USER", "u"),
                ("EMAIL_HOST_PASSWORD", "p")
            ])
            .is_ok()
        );
        // Credentials without encryption towards another machine: refused.
        let err = load(&[
            DB,
            ("EMAIL_HOST", "smtp.example.com"),
            ("EMAIL_USE_TLS", "False"),
            ("EMAIL_HOST_USER", "u"),
            ("EMAIL_HOST_PASSWORD", "p"),
        ])
        .unwrap_err()
        .to_string();
        assert!(err.contains("EMAIL_*") && !err.contains("\"p\""), "{err}");
        // Invalid values, all reported.
        let err = load(&[
            DB,
            ("EMAIL_HOST", "h"),
            ("EMAIL_PORT", "x"),
            ("EMAIL_USE_SSL", "peut-etre"),
            ("EMAIL_TIMEOUT_SECS", "0"),
        ])
        .unwrap_err()
        .to_string();
        assert!(
            err.contains("EMAIL_PORT")
                && err.contains("EMAIL_USE_SSL")
                && err.contains("EMAIL_TIMEOUT_SECS"),
            "{err}"
        );
        assert!(
            load(&[
                DB,
                ("EMAIL_HOST", "h"),
                ("EMAIL_USE_TLS", "true"),
                ("EMAIL_USE_SSL", "true")
            ])
            .unwrap_err()
            .to_string()
            .contains("mutually exclusive")
        );
        let c = load(&[DB, ("SMS_FREE_URL", "http://127.0.0.1:9/sendmsg")]).unwrap();
        assert_eq!(c.sms_free_url, "http://127.0.0.1:9/sendmsg");
    }

    #[test]
    fn the_solarmax_publisher_needs_an_explicit_host_and_validates_the_rest() {
        let c = load(&[DB, ("SOLARMAX_HOST", "127.0.0.1")]).unwrap();
        let s = c.solarmax.unwrap();
        assert_eq!(
            (
                s.host.as_str(),
                s.port,
                s.inverter,
                s.topic.as_str(),
                s.interval_secs
            ),
            (
                "127.0.0.1",
                12345,
                1,
                "solarmax/0x6f0930a64c8b8803/prod",
                10
            )
        );
        let c = load(&[
            DB,
            ("SOLARMAX_HOST", "onduleur.local"),
            ("SOLARMAX_PORT", "1234"),
            ("SOLARMAX_INVERTER_ID", "2"),
            ("SOLARMAX_TOPIC", "solarmax/essai/prod"),
            ("SOLARMAX_INTERVAL_SECS", "30"),
            ("OPEN_METEO_URL", "http://127.0.0.1:9999/v1/forecast"),
        ])
        .unwrap();
        let s = c.solarmax.unwrap();
        assert_eq!(
            (s.port, s.inverter, s.topic.as_str(), s.interval_secs),
            (1234, 2, "solarmax/essai/prod", 30)
        );
        assert_eq!(c.open_meteo_url, "http://127.0.0.1:9999/v1/forecast");
        for (key, bad) in [
            ("SOLARMAX_PORT", "abc"),
            ("SOLARMAX_INVERTER_ID", "0"),
            ("SOLARMAX_INVERTER_ID", "100"),
            ("SOLARMAX_TOPIC", "solarmax/+/prod"),
            ("SOLARMAX_INTERVAL_SECS", "0"),
        ] {
            let e = load(&[DB, ("SOLARMAX_HOST", "x"), (key, bad)]).unwrap_err();
            assert!(e.to_string().contains(key), "{key}={bad} : {e}");
        }
        // Without a host, the other SOLARMAX_* variables have no effect.
        assert!(
            load(&[DB, ("SOLARMAX_PORT", "abc")])
                .unwrap()
                .solarmax
                .is_none()
        );
    }

    #[test]
    fn full_configuration_is_parsed() {
        let c = load(&[
            DB,
            ("HTTP_ADDR", "127.0.0.1:9000"),
            ("ALLOWED_HOSTS", "dominet.local, 192.168.1.2 ,"),
            ("COOKIE_SECURE", "false"),
            ("LOG_FORMAT", "json"),
            ("MQTT_HOST", "mosquitto"),
            ("MQTT_PORT", "1884"),
            ("MQTT_USER", "dominet"),
            ("MQTT_PASSWORD", "s3cret"),
            ("REDUCTSTORE_URL", "http://reductstore:8383"),
            ("REDUCTSTORE_TOKEN", "tok"),
            ("IOT_QUOTA_GB", "8"),
            ("REDUCTSTORE_IOT_BUCKET", "iot-essai"),
            ("HISTORY_RETENTION_DAYS", "90"),
        ])
        .unwrap();
        assert_eq!(c.http_addr.port(), 9000);
        assert_eq!(c.allowed_hosts, ["dominet.local", "192.168.1.2"]);
        assert!(!c.cookie_secure);
        assert_eq!(c.log_format, LogFormat::Json);
        let m = c.mqtt.unwrap();
        assert_eq!(
            (m.host.as_str(), m.port, m.user.as_deref()),
            ("mosquitto", 1884, Some("dominet"))
        );
        assert_eq!(m.password.unwrap().expose(), "s3cret");
        assert_eq!(m.client_id, "dominet");
        assert_eq!(c.history_retention_days, 90);
        let r = c.reductstore.unwrap();
        assert_eq!(
            (r.url.as_str(), r.iot_quota_gb, r.iot_bucket.as_str()),
            ("http://reductstore:8383", 8, "iot-essai")
        );
    }

    #[test]
    fn all_errors_are_reported_together() {
        let e = load(&[
            ("HTTP_ADDR", "n'importe quoi"),
            ("COOKIE_SECURE", "peut-etre"),
            ("LOG_FORMAT", "xml"),
            ("MQTT_HOST", "m"),
            ("MQTT_PORT", "abc"),
            ("REDUCTSTORE_URL", "ftp://x"),
        ])
        .unwrap_err();
        assert_eq!(e.0.len(), 6, "{e}");
        for key in [
            "HTTP_ADDR",
            "DATABASE_URL",
            "COOKIE_SECURE",
            "LOG_FORMAT",
            "MQTT_PORT",
            "REDUCTSTORE_URL",
        ] {
            assert!(
                e.0.iter().any(|m| m.starts_with(key)),
                "{key} manquant dans {e}"
            );
        }
    }

    #[test]
    fn session_and_proxy_settings() {
        let d = load(&[DB]).unwrap();
        assert_eq!(d.site_root, "target/site");
        assert_eq!(
            load(&[DB, ("SITE_ROOT", "/app/site")]).unwrap().site_root,
            "/app/site"
        );
        assert_eq!((d.session_idle_minutes, d.session_max_days), (480, 14));
        assert_eq!(d.ws_heartbeat_secs, 25);
        assert_eq!(d.system_stats_secs, 5);
        assert_eq!(
            load(&[DB, ("WS_HEARTBEAT_SECS", "2")])
                .unwrap()
                .ws_heartbeat_secs,
            2
        );
        assert!(load(&[DB, ("WS_HEARTBEAT_SECS", "0")]).is_err());
        assert_eq!(d.trusted_proxies.len(), 0);
        let c = load(&[
            DB,
            ("SESSION_IDLE_MINUTES", "30"),
            ("SESSION_MAX_DAYS", "7"),
            ("TRUSTED_PROXIES", "172.18.0.1, 10.0.0.0/8, ::1"),
        ])
        .unwrap();
        assert_eq!((c.session_idle_minutes, c.session_max_days), (30, 7));
        let nets: Vec<String> = c.trusted_proxies.iter().map(ToString::to_string).collect();
        assert_eq!(nets, ["172.18.0.1/32", "10.0.0.0/8", "::1/128"]);
        for bad in [
            ("SESSION_IDLE_MINUTES", "0"),
            ("SESSION_MAX_DAYS", "abc"),
            ("SESSION_MAX_DAYS", "9999"),
            ("TRUSTED_PROXIES", "pas-une-ip"),
        ] {
            assert!(load(&[DB, bad]).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn the_master_key_must_be_32_bytes_of_base64() {
        assert!(
            load(&[
                DB,
                (
                    "DOMINET_MASTER_KEY",
                    "MDEyMzQ1Njc4OWFiY2RlZjAxMjM0NTY3ODlhYmNkZWY="
                )
            ])
            .unwrap()
            .master_key
            .is_some()
        );
        assert!(load(&[DB]).unwrap().master_key.is_none());
        for bad in ["maitre-secret", "AAAA", "pas du base64 !!"] {
            let e = load(&[DB, ("DOMINET_MASTER_KEY", bad)]).unwrap_err();
            assert!(e.0[0].starts_with("DOMINET_MASTER_KEY"), "{e}");
            assert!(
                !e.to_string().contains(bad),
                "la valeur ne doit pas etre rappelee dans l'erreur"
            );
        }
    }

    #[test]
    fn database_url_must_be_postgres() {
        assert!(load(&[("DATABASE_URL", "mysql://x")]).is_err());
        assert!(load(&[("DATABASE_URL", "postgresql://u@h/db")]).is_ok());
    }

    #[test]
    fn blank_values_count_as_unset() {
        let c = load(&[DB, ("MQTT_HOST", "   "), ("HTTP_ADDR", "")]).unwrap();
        assert!(c.mqtt.is_none());
        assert_eq!(c.http_addr.port(), 8000);
    }

    #[test]
    fn metrics_listener_defaults_to_9100_and_can_be_turned_off() {
        let with = |v: Option<&str>| {
            Config::from_lookup(|k| match k {
                "DATABASE_URL" => Some("postgres://u:p@h/db".into()),
                "METRICS_ADDR" => v.map(str::to_string),
                _ => None,
            })
        };
        assert_eq!(
            with(None).unwrap().metrics_addr,
            Some(SocketAddr::from(([0, 0, 0, 0], 9100)))
        );
        assert_eq!(
            with(Some("127.0.0.1:9200"))
                .unwrap()
                .metrics_addr
                .unwrap()
                .port(),
            9200
        );
        assert_eq!(with(Some("off")).unwrap().metrics_addr, None);
        assert!(with(Some("n'importe quoi")).is_err());
    }

    #[test]
    fn the_mediamtx_service_credential_is_both_or_none_and_stays_secret() {
        assert!(load(&[DB]).unwrap().mediamtx_read.is_none());
        let c = load(&[
            DB,
            ("MEDIAMTX_READ_USER", "dominet-video"),
            ("MEDIAMTX_READ_PASSWORD", "pw-secret"),
        ])
        .unwrap();
        let read = c.mediamtx_read.as_ref().unwrap();
        assert_eq!(
            (read.user.as_str(), read.password.expose()),
            ("dominet-video", "pw-secret")
        );
        assert!(!format!("{c:?}").contains("pw-secret"));
        for only in [("MEDIAMTX_READ_USER", "u"), ("MEDIAMTX_READ_PASSWORD", "p")] {
            let e = load(&[DB, only]).unwrap_err().to_string();
            assert!(e.contains("set both or none"), "{e}");
        }
    }

    #[test]
    fn secrets_never_appear_in_debug_output() {
        let c = load(&[
            ("DATABASE_URL", "postgres://u:motdepasse-secret@h/db"),
            ("MQTT_HOST", "m"),
            ("MQTT_PASSWORD", "mqtt-secret"),
            ("REDUCTSTORE_URL", "http://r"),
            ("REDUCTSTORE_TOKEN", "token-secret"),
            (
                "DOMINET_MASTER_KEY",
                "MDEyMzQ1Njc4OWFiY2RlZjAxMjM0NTY3ODlhYmNkZWY=",
            ),
        ])
        .unwrap();
        let dump = format!("{c:?}");
        for secret in [
            "motdepasse-secret",
            "mqtt-secret",
            "token-secret",
            "maitre-secret",
        ] {
            assert!(!dump.contains(secret), "{secret} fuit dans {dump}");
        }
        assert!(dump.contains("Secret(***)"));
    }
}
