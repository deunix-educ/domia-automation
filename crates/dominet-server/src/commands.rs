// SPDX-License-Identifier: AGPL-3.0-or-later
//! Administration commands (outside `serve`).

use anyhow::{Context, bail};
use dominet_domain::repo::UserRepo;
use dominet_domain::user::{Role, User};
use dominet_store::PgStore;
use sqlx::postgres::PgPoolOptions;

use crate::auth::password::Hasher;
use crate::auth::service::{AuthError, AuthService};
use crate::cli::{
    AggregateEnergyArgs, DumpContextArgs, NotifyTestArgs, PublishArgs, PublishTarget, RestoreArgs,
    SeedArgs, SeedWeatherArgs, SuperuserArgs, UserArgs,
};
use crate::error::conflict_message;

use crate::config::Config;
use crate::server::lazy_pool;

/// `dominet migrate`: applies the pending migrations and prints the summary.
///
/// # Errors
/// Returns an error if the database is unreachable or if a migration fails.
pub async fn migrate(config: &Config) -> anyhow::Result<()> {
    let pool = lazy_pool(config)?;
    // The tracking table does not exist before the first migration: we then count 0.
    let before: i64 = sqlx::query_scalar("SELECT count(*) FROM _sqlx_migrations")
        .fetch_one(&pool)
        .await
        .unwrap_or(0);
    dominet_store::migrate(&pool)
        .await
        .context("migration impossible")?;
    let total = dominet_store::MIGRATOR.iter().count();
    let applied = total.saturating_sub(usize::try_from(before).unwrap_or(0));
    tracing::info!(applied, total, "migrations finished");
    println!("{applied} migration(s) applied; schema up to date ({total} in total).");
    Ok(())
}

/// Environment variable to supply the password without prompting (scripts, Docker).
pub const PASSWORD_ENV: &str = "DOMINET_PASSWORD";

async fn connect(config: &Config) -> anyhow::Result<PgStore> {
    let pool = PgPoolOptions::new()
        .max_connections(2)
        .connect(config.database_url.expose())
        .await
        .context("cannot connect to the database (migrations applied? `dominet migrate`)")?;
    Ok(PgStore::new(pool))
}

fn auth_service(store: PgStore) -> AuthService {
    AuthService::new(
        store,
        Hasher::new(),
        dominet_domain::auth::LockoutPolicy::default(),
    )
}

/// Readable message for the CLI (conflicts name an SQL constraint: we translate them).
fn explain(err: &AuthError) -> anyhow::Error {
    match err {
        AuthError::Repo(dominet_domain::RepoError::Conflict { constraint }) => {
            anyhow::anyhow!("{}", conflict_message(constraint))
        }
        AuthError::Repo(dominet_domain::RepoError::NotFound) => {
            anyhow::anyhow!("account not found")
        }
        other => anyhow::anyhow!("{other}"),
    }
}

fn ask(prompt: &str) -> anyhow::Result<String> {
    use std::io::Write;
    print!("{prompt}");
    std::io::stdout().flush()?;
    let mut line = String::new();
    std::io::stdin().read_line(&mut line)?;
    Ok(line.trim().to_string())
}

/// Password: environment variable if present, otherwise masked input asked twice.
fn read_password() -> anyhow::Result<String> {
    if let Ok(pw) = std::env::var(PASSWORD_ENV) {
        return Ok(pw);
    }
    let first = rpassword::prompt_password("Password: ")?;
    let second = rpassword::prompt_password("Password (confirmation): ")?;
    if first != second {
        bail!("the two passwords differ");
    }
    Ok(first)
}

/// Core of `createsuperuser`, separated from the input to be testable.
///
/// # Errors
/// Name or password refused by the policy, or name already taken.
pub async fn create_superuser(
    auth: &AuthService,
    username: &str,
    email: &str,
    password: &str,
) -> Result<User, AuthError> {
    auth.create_user(username, email, "", Role::Admin, password)
        .await
}

/// Core of `passwd`: changes the password and revokes the account's sessions.
///
/// # Errors
/// `Repo(NotFound)` if the account does not exist, `Invalid` if the password violates the policy.
pub async fn change_password(
    auth: &AuthService,
    store: &PgStore,
    username: &str,
    password: &str,
) -> Result<User, AuthError> {
    let user = store
        .users()
        .find_by_username(username)
        .await?
        .ok_or(dominet_domain::RepoError::NotFound)?;
    auth.set_password(user.id, &user.username, password).await?;
    Ok(user)
}

/// `dominet createsuperuser`.
///
/// # Errors
/// See [`create_superuser`]; also an error if the database is unreachable.
pub async fn createsuperuser(config: &Config, args: SuperuserArgs) -> anyhow::Result<()> {
    let store = connect(config).await?;
    let username = match args.username {
        Some(u) => u,
        None => ask("User name: ")?,
    };
    let password = read_password()?;
    let user = create_superuser(&auth_service(store), &username, &args.email, &password)
        .await
        .map_err(|e| explain(&e))?;
    println!("Administrator account \"{}\" created.", user.username);
    Ok(())
}

/// `dominet passwd`.
///
/// # Errors
/// See [`change_password`].
pub async fn passwd(config: &Config, args: UserArgs) -> anyhow::Result<()> {
    let store = connect(config).await?;
    let username = match args.username {
        Some(u) => u,
        None => ask("User name: ")?,
    };
    let password = read_password()?;
    let user = change_password(&auth_service(store.clone()), &store, &username, &password)
        .await
        .map_err(|e| explain(&e))?;
    println!(
        "Password of \"{}\" changed; its sessions were closed.",
        user.username
    );
    Ok(())
}

/// `dominet healthcheck`: `GET /healthz` on the local server; true if the response is 200.
///
/// Without `--addr`, the port of `HTTP_ADDR` (default 8000) on 127.0.0.1. Reads neither the database nor the full configuration.
#[must_use]
pub fn healthcheck(addr: Option<&str>) -> bool {
    use std::io::{Read, Write};
    use std::net::{SocketAddr, TcpStream, ToSocketAddrs};
    use std::time::Duration;

    let target = addr.map_or_else(
        || {
            let port = std::env::var("HTTP_ADDR")
                .ok()
                .and_then(|a| a.parse::<SocketAddr>().ok())
                .map_or(8000, |a| a.port());
            format!("127.0.0.1:{port}")
        },
        str::to_owned,
    );
    let Some(sock) = target.to_socket_addrs().ok().and_then(|mut a| a.next()) else {
        return false;
    };
    let timeout = Duration::from_secs(3);
    let Ok(mut stream) = TcpStream::connect_timeout(&sock, timeout) else {
        return false;
    };
    let _ = stream.set_read_timeout(Some(timeout));
    let request = format!("GET /healthz HTTP/1.0\r\nHost: {target}\r\n\r\n");
    if stream.write_all(request.as_bytes()).is_err() {
        return false;
    }
    let mut head = [0_u8; 32];
    let n = stream.read(&mut head).unwrap_or(0);
    head[..n].starts_with(b"HTTP/1.1 200") || head[..n].starts_with(b"HTTP/1.0 200")
}

/// `dominet gen-master-key`: prints a new master key (no configuration needed).
pub fn gen_master_key() {
    println!("{}", crate::secrets::MasterKey::generate());
    eprintln!(
        "Copy into DOMINET_MASTER_KEY (the .env file). Losing this key makes the stored secrets unreadable."
    );
}

/// Summary of `seed`.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct SeedSummary {
    pub settings_created: usize,
    pub settings_existing: usize,
    pub navbar_created: usize,
    pub navbar_existing: usize,
    /// Shipped Prometheus and Grafana alert rules, created (R7); never recreated if the administrator deleted them.
    pub rules_created: usize,
}

/// Core of `seed`, separated from the configuration to be testable: settings then navigation, without overwriting anything.
///
/// # Errors
/// See [`crate::settings::SettingsService::seed`]; repository error for the navigation.
pub async fn seed_data(
    settings: &crate::settings::SettingsService,
    store: &PgStore,
    lookup: impl Fn(&str) -> Option<String>,
    dry_run: bool,
) -> anyhow::Result<SeedSummary> {
    use dominet_domain::registry::default_navbar;
    use dominet_domain::repo::NavbarRepo;

    let report = settings
        .seed(lookup, dry_run)
        .await
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    let navbar = store.navbar();
    let existing = navbar.list().await?;
    let mut summary = SeedSummary {
        settings_created: report.created.len(),
        settings_existing: report.existing.len(),
        ..SeedSummary::default()
    };
    // The default navigation is only created if the table is empty: we do not resurrect an entry that
    // the administrator deleted or renamed.
    let defaults = default_navbar();
    for item in &defaults {
        item.validate()
            .map_err(|e| anyhow::anyhow!("invalid default navigation: {e}"))?;
    }
    if existing.is_empty() {
        for item in &defaults {
            if !dry_run {
                navbar.insert_if_absent(item).await?;
            }
        }
        summary.navbar_created = defaults.len();
    } else {
        summary.navbar_existing = existing.len();
    }
    summary.rules_created = seed_rules(store, dry_run).await?;
    Ok(summary)
}

/// Shipped alert rules: created **only in an empty table** (like the navigation, we do not resurrect a rule that
/// the administrator deleted). Returns the number of rules created (or to create in a simulation).
async fn seed_rules(store: &PgStore, dry_run: bool) -> anyhow::Result<usize> {
    use dominet_domain::observability::{default_grafana_rules, default_prometheus_rules};
    use dominet_domain::repo::{GrafanaRuleRepo, PrometheusRuleRepo};

    let mut created = 0;
    let prom = store.prometheus_rules();
    if prom.list().await?.is_empty() {
        for rule in default_prometheus_rules() {
            if let Some((field, _)) = rule.problems().first() {
                anyhow::bail!("invalid shipped Prometheus rule ({}): {field}", rule.name);
            }
            if dry_run || prom.insert_if_absent(&rule).await? {
                created += 1;
            }
        }
    }
    let graf = store.grafana_rules();
    if graf.list().await?.is_empty() {
        for rule in default_grafana_rules() {
            if let Some((field, _)) = rule.problems().first() {
                anyhow::bail!("invalid shipped Grafana rule ({}): {field}", rule.uid);
            }
            if dry_run || graf.insert_if_absent(&rule).await? {
                created += 1;
            }
        }
    }
    Ok(created)
}

/// `dominet sync-rules`: writes `rules.yml` and `rules.yaml` from the database and asks for their reload.
///
/// # Errors
/// Database unreachable, or one of the two services failing (the exit code says so).
pub async fn sync_rules(config: &Config) -> anyhow::Result<()> {
    use crate::rules::SyncStatus;

    let store = connect(config).await?;
    let state = crate::AppState::new(store.pool().clone(), std::sync::Arc::new(config.clone()));
    let (prom, graf) = state.rules.sync_all().await;
    let mut failed = false;
    for (name, status) in [("Prometheus", &prom), ("Grafana", &graf)] {
        match status {
            SyncStatus::Disabled => println!("{name}: not configured (file path missing)."),
            SyncStatus::Done { groups, reloaded } => println!(
                "{name}: {groups} group(s) written, reload {}.",
                match reloaded {
                    Some(true) => "requested",
                    Some(false) => "FAILED",
                    None => "to do by hand (address missing)",
                }
            ),
            SyncStatus::Failed(why) => println!("{name}: FAILED ({why})."),
        }
        failed |= !status.is_ok();
    }
    if failed {
        anyhow::bail!("rule synchronisation incomplete");
    }
    Ok(())
}

/// Labels to create for the devices' state keys: those of the known reference list (`KNOWN_LABELS`) or, failing that, a neutral icon
/// and the key as the name. Without `dry_run`, the labels are written (never overwriting an existing one).
///
/// # Errors
/// Repository error.
pub async fn seed_labels_data(
    store: &PgStore,
    dry_run: bool,
) -> anyhow::Result<Vec<dominet_domain::device::DeviceLabelDraft>> {
    use dominet_domain::device::known_label;
    use dominet_domain::repo::DeviceLabelRepo;

    let labels = store.device_labels();
    let existing: std::collections::HashSet<String> =
        labels.list().await?.into_iter().map(|l| l.key).collect();
    let missing: Vec<_> = labels
        .state_keys()
        .await?
        .into_iter()
        .filter(|k| !existing.contains(k))
        .map(|k| known_label(&k))
        .collect();
    if !dry_run {
        labels.create_missing(&missing).await?;
    }
    Ok(missing)
}

/// `dominet seed-labels [--dry-run]`.
///
/// # Errors
/// See [`seed_labels_data`].
pub async fn seed_labels(config: &Config, args: SeedArgs) -> anyhow::Result<()> {
    let store = connect(config).await?;
    let missing = seed_labels_data(&store, args.dry_run).await?;
    if missing.is_empty() {
        println!("No missing key.");
        return Ok(());
    }
    for l in &missing {
        println!("  {:30} {:25} {:30} {}", l.key, l.icon_name, l.name, l.unit);
    }
    let verb = if args.dry_run {
        "to create (simulation)"
    } else {
        "created"
    };
    println!("{} entree(s) {verb}", missing.len());
    Ok(())
}

/// `dominet seed [--dry-run]`: the initial values come from the environment (same names as the settings),
/// otherwise from default values. The values are never printed.
///
/// # Errors
/// See [`seed_data`].
pub async fn seed(config: &Config, args: SeedArgs) -> anyhow::Result<()> {
    let store = connect(config).await?;
    let state = crate::AppState::new(store.pool().clone(), std::sync::Arc::new(config.clone()));
    let summary = seed_data(
        &state.settings,
        &store,
        |k| std::env::var(k).ok(),
        args.dry_run,
    )
    .await?;
    let mode = if args.dry_run { "(simulation) " } else { "" };
    println!(
        "{mode}settings: {} created, {} existing; navigation: {} created, {} existing; shipped alert rules: {} created.",
        summary.settings_created,
        summary.settings_existing,
        summary.navbar_created,
        summary.navbar_existing,
        summary.rules_created
    );
    Ok(())
}

// ------------------------------------------------------------------ weather, publishers, context (R3 lot 6)

/// `dominet seed-weather`: "Meteo" MQTT source (`meteo/#`, automatic devices), `meteo_locale` device, and the station if
/// `--lat` and `--lon` are given. Never replaces what exists. Unlike the reference implementation, no fake initial state is
/// written: the device waits for the first real reading.
///
/// # Errors
/// Database unreachable, or invalid station parameters.
pub async fn seed_weather(config: &Config, args: SeedWeatherArgs) -> anyhow::Result<()> {
    use dominet_domain::device::{DeviceDraft, DeviceType, MqttSource, WeatherStation};
    use dominet_domain::repo::{DeviceRepo, MqttSourceRepo, WeatherRepo};

    let store = connect(config).await?;
    let mark = |created: bool| {
        if args.dry_run {
            "[DRY]"
        } else if created {
            "[OK] "
        } else {
            "[--]"
        }
    };

    let sources = store.mqtt_sources();
    let missing = !sources.list().await?.iter().any(|s| s.name == "Météo");
    if missing && !args.dry_run {
        let mut s = MqttSource::draft("Météo", "meteo/#");
        s.base_topic = "meteo".into();
        s.auto_device = true;
        s.auto_device_type = Some(DeviceType::Sensor);
        s.description = "Station météo Open-Meteo (topic : meteo/meteo_locale)".into();
        sources.create(&s).await?;
    }
    println!("  {} Source MQTT : Météo (meteo/#)", mark(missing));

    let devices = store.devices();
    let missing = devices.find_by_name("meteo_locale").await?.is_none();
    if missing && !args.dry_run {
        let mut d = DeviceDraft::new("meteo_locale");
        d.device_type = DeviceType::Sensor;
        d.base_topic = "meteo".into();
        d.description = "Local weather through Open-Meteo (free API, no key)".into();
        d.icon = "cloud-sun".into();
        d.tracked_keys = ["temperature", "humidity", "precipitation", "wind_speed"]
            .map(String::from)
            .to_vec();
        d.hidden_keys = [
            "weather_code",
            "uv_index",
            "feels_like",
            "cloud_cover",
            "wind_direction",
            "pressure",
        ]
        .map(String::from)
        .to_vec();
        devices.create(&d).await?;
    }
    println!("  {} Device: meteo_locale", mark(missing));

    match (args.lat, args.lon) {
        (Some(lat), Some(lon)) => {
            let stations = store.weather();
            let missing = !stations
                .list()
                .await?
                .iter()
                .any(|s| s.mqtt_topic_suffix == "meteo_locale");
            if missing {
                let mut s = WeatherStation::draft("Météo locale", lat, lon);
                s.altitude = args.alt;
                s.timezone = args.timezone.clone();
                s.refresh_minutes = args.refresh;
                s.description = format!("Station Open-Meteo — {lat:.4}, {lon:.4}");
                if let Some((field, problem)) = s.problems().first() {
                    bail!("invalid station ({field}: {problem:?})");
                }
                if !args.dry_run {
                    stations.create(&s).await?;
                }
            }
            println!(
                "  {} Station : Météo locale ({lat:.4}, {lon:.4})",
                mark(missing)
            );
        }
        _ => println!("  [--] Station skipped — pass --lat and --lon to create it"),
    }
    Ok(())
}

/// MQTT client of a diagnostic: waits for the connection (10 s at most).
async fn diagnostic_client(config: &Config) -> anyhow::Result<dominet_mqtt::MqttClient> {
    let mut cfg = config
        .mqtt
        .clone()
        .context("MQTT_HOST is not set (use --print-only to avoid publishing)")?;
    cfg.client_id = format!("{}-diagnostic", cfg.client_id);
    let client = dominet_mqtt::start(&crate::mqtt::settings(&cfg)).client;
    let mut status = client.watch_status();
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        while *status.borrow_and_update() != dominet_mqtt::Status::Connected {
            if status.changed().await.is_err() {
                break;
            }
        }
    })
    .await
    .context("MQTT broker unreachable")?;
    Ok(client)
}

/// `dominet publish weather|solarmax [--print-only]`: a single cycle, to check an installation.
///
/// # Errors
/// Source unreachable or unreadable, broker unreachable (except `--print-only`), setting missing.
pub async fn publish(config: &Config, args: PublishArgs) -> anyhow::Result<()> {
    use dominet_domain::repo::WeatherRepo;
    use dominet_integrations::solarmax::SolarMaxClient;
    use dominet_integrations::weather::OpenMeteo;

    let client = if args.print_only {
        None
    } else {
        Some(diagnostic_client(config).await?)
    };
    let publish = |topic: &str, payload: &str| -> anyhow::Result<()> {
        if let Some(c) = &client {
            c.publish(topic, payload, 1, false)
                .map_err(|e| anyhow::anyhow!("publication refused: {e}"))?;
            println!("-> published on {topic}");
        }
        Ok(())
    };
    match args.target {
        PublishTarget::Weather => {
            let store = connect(config).await?;
            let api = OpenMeteo::new(&config.open_meteo_url)?;
            let stations = store.weather().list_enabled().await?;
            if stations.is_empty() {
                println!(
                    "No weather station enabled (see /admin/weather-stations or `dominet seed-weather`)."
                );
            }
            for station in stations {
                let data = api
                    .fetch(&station)
                    .await
                    .with_context(|| format!("station {}", station.name))?;
                println!(
                    "{} : {}",
                    station.name,
                    serde_json::to_string_pretty(&data)?
                );
                publish(&station.mqtt_topic(), &data.to_string())?;
            }
        }
        PublishTarget::Solarmax => {
            let cfg = config.solarmax.clone().context(
                "SOLARMAX_HOST is not set: the inverter address has no default value (never contacted by accident)",
            )?;
            let settings = crate::publishers::solarmax_settings(&cfg);
            let mut inverter = SolarMaxClient::new(&settings.host, settings.port, settings.timeout);
            let production = inverter.read_production(settings.inverter).await?;
            let json = production.to_json();
            println!("{}", serde_json::to_string_pretty(&json)?);
            publish(&settings.topic, &json.to_string())?;
        }
    }
    if let Some(c) = &client {
        // The publication is queued: we let the messages go out before cutting.
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        c.shutdown();
    }
    Ok(())
}

/// `context.yaml` document: active devices (by type then name), empty lists for what arrives in R4, reference schemas.
///
/// # Errors
/// Database unreachable.
#[allow(clippy::too_many_lines)] // one section per collection,
pub async fn context_document(store: &PgStore) -> anyhow::Result<serde_json::Value> {
    use dominet_domain::context::{numeric_keys, reference_schemas, state_schema};
    use dominet_domain::repo::{
        AlertRuleRepo, AutomationRepo, DeviceFilter, DeviceRepo, GrafanaRuleRepo, LocationRepo,
        SceneRepo,
    };
    use serde_json::{Map, Value, json};

    let locations: std::collections::HashMap<i64, String> = store
        .locations()
        .list()
        .await?
        .into_iter()
        .map(|l| (l.id.0, l.name))
        .collect();
    let mut devices = store
        .devices()
        .list(&DeviceFilter {
            active_only: true,
            ..DeviceFilter::default()
        })
        .await?;
    devices.sort_by(|a, b| {
        (a.device_type.as_str(), &a.friendly_name).cmp(&(b.device_type.as_str(), &b.friendly_name))
    });
    let names: std::collections::HashMap<i64, String> = store
        .devices()
        .list(&DeviceFilter::default())
        .await?
        .into_iter()
        .map(|d| (d.id.0, d.friendly_name))
        .collect();
    let device_name =
        |id: dominet_domain::device::DeviceId| names.get(&id.0).cloned().unwrap_or_default();
    let mut entries = Map::new();
    for d in devices {
        let schema = state_schema(&d.state);
        let mut entry = Map::new();
        entry.insert("type".into(), json!(d.device_type.as_str()));
        entry.insert(
            "state_schema".into(),
            Value::Object(schema.iter().map(|(k, t)| (k.clone(), json!(t))).collect()),
        );
        entry.insert("command_topic".into(), json!(d.command_topic()));
        let numeric = numeric_keys(&schema);
        if !numeric.is_empty() {
            entry.insert("numeric_keys".into(), json!(numeric));
        }
        if let Some(name) = d.location.and_then(|l| locations.get(&l.0)) {
            entry.insert("location".into(), json!(name));
        }
        entries.insert(d.friendly_name, Value::Object(entry));
    }
    // Active Grafana rules, by source then identifier (like the reference).
    let mut grafana: Vec<_> = store
        .grafana_rules()
        .list()
        .await?
        .into_iter()
        .filter(|r| r.draft.active)
        .collect();
    grafana.sort_by(|a, b| {
        (&a.draft.datasource_uid, &a.draft.uid).cmp(&(&b.draft.datasource_uid, &b.draft.uid))
    });
    let grafana: Vec<Value> = grafana
        .into_iter()
        .map(|r| {
            let d = r.draft;
            json!({
                "uid": d.uid,
                "title": d.title,
                "datasource": d.datasource_uid,
                "expr": d.expr,
                "for_duration": d.for_duration,
                "severity": d.severity.as_str(),
                "no_data_state": d.no_data_state,
            })
        })
        .collect();
    // Active scenes by identifier, with their commands in publication order.
    let mut scenes: Vec<_> = store
        .scenes()
        .list()
        .await?
        .into_iter()
        .filter(|s| s.active)
        .collect();
    scenes.sort_by_key(|s| s.id.0);
    let scenes: Vec<Value> = scenes
        .into_iter()
        .map(|s| {
            let actions: Vec<Value> = s
                .actions
                .iter()
                .map(|a| json!({"device": device_name(a.device), "payload": a.payload}))
                .collect();
            json!({"pk": s.id.0, "name": s.name, "icon": s.icon, "actions": actions})
        })
        .collect();
    // Enabled automations by trigger type then name; the condition and the Rhai code, specific to
    // DomiNet, are only written if there are any.
    let mut automations: Vec<_> = store.automations().list_enabled().await?;
    automations.sort_by(|a, b| {
        (a.draft.trigger.kind(), &a.draft.name).cmp(&(b.draft.trigger.kind(), &b.draft.name))
    });
    let automations: Vec<Value> = automations
        .into_iter()
        .map(|a| {
            let parts = a.draft.parts();
            let mut entry = Map::new();
            entry.insert("name".into(), json!(a.draft.name));
            entry.insert("trigger_type".into(), json!(parts.trigger_type));
            entry.insert("trigger_config".into(), parts.trigger_config);
            if parts.condition.as_object().is_some_and(|c| !c.is_empty()) {
                entry.insert("condition".into(), parts.condition);
            }
            entry.insert("action_type".into(), json!(parts.action_type));
            entry.insert("action_config".into(), parts.action_config);
            if !a.draft.script.is_empty() {
                entry.insert("script".into(), json!(a.draft.script));
            }
            Value::Object(entry)
        })
        .collect();
    // Enabled device alert rules, by device.
    let mut rules: Vec<_> = store
        .alert_rules()
        .list()
        .await?
        .into_iter()
        .filter(|r| r.rule.enabled)
        .collect();
    rules.sort_by(|a, b| {
        (device_name(a.rule.device), &a.rule.name).cmp(&(device_name(b.rule.device), &b.rule.name))
    });
    let alert_rules: Vec<Value> = rules
        .into_iter()
        .map(|r| {
            let d = r.rule;
            json!({
                "name": d.name,
                "device": device_name(d.device),
                "key": d.key,
                "operator": d.operator.as_str(),
                "threshold_num": d.threshold_num,
                "threshold_str": if d.threshold_str.is_empty() { Value::Null } else { json!(d.threshold_str) },
                "severity": d.severity.as_str(),
                "cooldown_minutes": d.cooldown_minutes,
                "notify": {"email": d.notify_email, "sms": d.notify_sms, "android": d.notify_android},
            })
        })
        .collect();
    Ok(json!({
        "devices": entries,
        "scenes": scenes,
        "automations": automations,
        "alert_rules": alert_rules,
        "grafana_rules": grafana,
        "schemas": reference_schemas(),
    }))
}

/// `dominet dump-context [--output FILE]`.
///
/// # Errors
/// Database unreachable or file not writable.
pub async fn dump_context(config: &Config, args: DumpContextArgs) -> anyhow::Result<()> {
    let store = connect(config).await?;
    let document = context_document(&store).await?;
    let text = format!(
        "# context.yaml — DomiNet snapshot: active devices, scenes, automations and alert rules, Grafana rules, reference schemas.\n{}",
        dominet_domain::context::to_yaml(&document)
    );
    match args.output {
        Some(path) => {
            std::fs::write(&path, text)
                .with_context(|| format!("cannot write {}", path.display()))?;
            println!("context.yaml written → {}", path.display());
        }
        None => print!("{text}"),
    }
    Ok(())
}

// ------------------------------------------------------------------ energy (R4 lot 7)

/// Result of `seed-energy` for a device: `None` if it does not exist, otherwise the keys added to the tracking (empty: already tracked).
pub type SeedEnergyLine = (String, Option<Vec<String>>);

/// Adds to the `tracked_keys` of the summary's three devices the keys the computation needs (`HCHP`, `HCHC`, `qdy`, `energy_a`,
/// `energy_b`) and the powers of the Grafana "Energy" dashboard (`PAPP`, `pac`, `power_ab`). Removes nothing; without `dry_run` nothing is written.
///
/// # Errors
/// Database unreachable.
pub async fn seed_energy_data(
    store: &PgStore,
    names: &dominet_automation::energy::EnergyDevices,
    dry_run: bool,
) -> anyhow::Result<Vec<SeedEnergyLine>> {
    use dominet_automation::energy::required_keys;
    use dominet_domain::device::DeviceDraft;
    use dominet_domain::repo::DeviceRepo;

    let devices = store.devices();
    let mut out = vec![];
    for (name, keys) in required_keys(names) {
        let Some(device) = devices.find_by_name(&name).await? else {
            out.push((name, None));
            continue;
        };
        let missing: Vec<String> = keys
            .iter()
            .filter(|k| !k.is_empty() && !device.tracked_keys.iter().any(|t| t == *k))
            .map(|k| (*k).to_string())
            .collect();
        if !missing.is_empty() && !dry_run {
            let mut draft = DeviceDraft::new(&device.friendly_name);
            draft.description.clone_from(&device.description);
            draft.device_type = device.device_type;
            draft.group = device.group;
            draft.location = device.location;
            draft.base_topic.clone_from(&device.base_topic);
            draft.active = device.active;
            draft.icon.clone_from(&device.icon);
            draft.image_url.clone_from(&device.image_url);
            draft.tracked_keys = device.tracked_keys.clone();
            draft.tracked_keys.extend(missing.iter().cloned());
            draft.hidden_keys.clone_from(&device.hidden_keys);
            draft.grafana_dashboard = device.grafana_dashboard;
            devices.update(device.id, &draft).await?;
        }
        out.push((name, Some(missing)));
    }
    Ok(out)
}

/// `dominet seed-energy`: see [`seed_energy_data`]. Without a history of these keys, the summary stays at zero.
///
/// # Errors
/// Database unreachable.
pub async fn seed_energy(config: &Config, args: SeedArgs) -> anyhow::Result<()> {
    let store = connect(config).await?;
    for (name, line) in seed_energy_data(&store, &config.energy, args.dry_run).await? {
        match line {
            None => {
                println!("  [--] {name}: device not found (it will be created by its MQTT source)");
            }
            Some(added) if added.is_empty() => println!("[--] {name}: keys already tracked"),
            Some(added) => println!(
                "  {} {name}: keys added to tracking: {}",
                if args.dry_run { "[DRY]" } else { "[OK] " },
                added.join(", ")
            ),
        }
    }
    Ok(())
}

/// `dominet aggregate-energy [--days N | --date YYYY-MM-DD]`: computes and records the summary of the last days (yesterday by default),
/// like the night job.
///
/// # Errors
/// Database unreachable.
pub async fn aggregate_energy(config: &Config, args: AggregateEnergyArgs) -> anyhow::Result<()> {
    use dominet_automation::energy::{compute_and_store, yesterday};

    let store = connect(config).await?;
    let tz: chrono_tz::Tz = config.timezone.parse().unwrap_or(chrono_tz::Europe::Paris);
    let days: Vec<chrono::NaiveDate> = if let Some(d) = args.date {
        vec![d]
    } else {
        let last = yesterday(tz, chrono::Utc::now());
        (0..i64::from(args.days))
            .map(|i| last - chrono::Duration::days(i))
            .collect()
    };
    for day in days {
        let d = compute_and_store(
            &store.devices(),
            &store.state_log(),
            &store.energy(),
            &config.energy,
            tz,
            day,
        )
        .await?;
        println!(
            "{day}: linky={}Wh solar={}Wh pac={}Wh net={:+}Wh",
            d.linky_total_wh(),
            d.solar_wh,
            d.pac_wh,
            d.net_wh()
        );
    }
    Ok(())
}

/// `dominet notify-test --email [--to ADDRESS] --sms`: **really** sends a test message through each requested channel, with the
/// environment's configuration and the settings. It is the only real sending path outside alerts and messaging (plan R5, D1):
/// it only runs on explicit request.
///
/// # Errors
/// No channel requested, database unreachable or a sending failure (the secret never appears in the message).
pub async fn notify_test(config: &Config, args: NotifyTestArgs) -> anyhow::Result<()> {
    use crate::notify::Channels;
    use crate::secrets::{MasterKey, SecretBox};
    use crate::settings::SettingsService;

    if !args.email && !args.sms {
        anyhow::bail!("nothing to send: add --email and/or --sms");
    }
    let store = connect(config).await?;
    let sealer = config
        .master_key
        .as_ref()
        .and_then(|k| MasterKey::parse(k.expose()).ok())
        .map(|k| SecretBox::new(&k));
    let channels = Channels::new(config, SettingsService::new(store.settings(), sealer));
    let now = chrono::Utc::now().format("%d/%m/%Y %H:%M:%S UTC");
    let mut failed = false;
    if args.email {
        let subject = "[DomiNet] Send test";
        let body = format!("DomiNet test message, {now}.");
        let result = if let Some(to) = &args.to {
            channels
                .send_email(to, subject, &body)
                .await
                .map(|()| to.clone())
        } else {
            async {
                let to = channels.alert_recipient().await?;
                channels.send_alert_email(subject, &body).await?;
                Ok::<_, dominet_notify::NotifyError>(to)
            }
            .await
        };
        match result {
            Ok(to) => println!("e-mail: sent to {to}"),
            Err(e) => {
                failed = true;
                eprintln!("e-mail: FAILED - {e}");
            }
        }
    }
    if args.sms {
        match channels
            .send_sms_free(&format!("[DomiNet] Send test {now}"))
            .await
        {
            Ok(()) => println!("Free Mobile SMS: sent"),
            Err(e) => {
                failed = true;
                eprintln!("Free Mobile SMS: FAILED - {e}");
            }
        }
    }
    if failed {
        anyhow::bail!("at least one send failed");
    }
    Ok(())
}

// ------------------------------------------------------------------ backups (R7 lot 4)

fn human_bytes(n: u64) -> String {
    #[allow(clippy::cast_precision_loss, reason = "affichage")]
    let mb = n as f64 / 1_048_576.0;
    format!("{mb:.1} Mo")
}

/// `dominet backup`: immediate backup.
///
/// # Errors
/// `BACKUP_DIR` missing, PostgreSQL client failing, disk full.
pub async fn backup(config: &Config) -> anyhow::Result<()> {
    let report = crate::backup::run_once(crate::backup::BackupService::new(config)).await?;
    println!(
        "Base : {} ({})",
        report.postgres.path.display(),
        human_bytes(report.postgres.bytes)
    );
    match &report.reductstore {
        Some(a) => println!(
            "ReductStore : {} ({})",
            a.path.display(),
            human_bytes(a.bytes)
        ),
        None => {
            println!("ReductStore: not backed up (REDUCTSTORE_BACKUP_SRC missing or not found).");
        }
    }
    println!("{} expired backup(s) deleted.", report.pruned);
    Ok(())
}

/// `dominet backups`: list of backups.
///
/// # Errors
/// `BACKUP_DIR` missing.
pub fn backups(config: &Config) -> anyhow::Result<()> {
    let service = crate::backup::BackupService::new(config);
    anyhow::ensure!(service.is_configured(), "BACKUP_DIR is not set");
    let all = service.list();
    if all.is_empty() {
        println!("No backup.");
    }
    for (kind, path) in all {
        let size = path.metadata().map_or(0, |m| m.len());
        println!("{kind:12} {:>10}  {}", human_bytes(size), path.display());
    }
    Ok(())
}

/// `dominet restore`: restores the database or the ReductStore volume.
///
/// # Errors
/// Wrong checksum, database unreachable, confirmation missing, non-empty target folder.
pub fn restore(config: &Config, args: RestoreArgs) -> anyhow::Result<()> {
    use crate::backup::BackupService;

    let service = BackupService::new(config);
    let file = match (&args.file, args.latest.as_deref()) {
        (Some(f), _) => f.clone(),
        (None, Some(kind)) => service
            .latest(kind)
            .with_context(|| format!("no `{kind}` backup in BACKUP_DIR"))?,
        (None, None) => anyhow::bail!("give a file or --latest postgres|reductstore"),
    };
    let name = file
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    if name.ends_with(".sql.gz") {
        anyhow::ensure!(
            args.yes,
            "the restore REPLACES the database data: rerun with --yes to confirm"
        );
        service.restore_postgres(&file)?;
        println!("Database restored from {}.", file.display());
        println!(
            "Restart DomiNet; encrypted secrets are only readable with the original DOMINET_MASTER_KEY."
        );
    } else if name.ends_with(".tar.gz") {
        let dest = args
            .to
            .context("--to <ReductStore data folder> is mandatory")?;
        let n = BackupService::restore_reductstore(&file, &dest, args.force)?;
        println!(
            "{n} item(s) extracted into {} (start ReductStore on this folder).",
            dest.display()
        );
    } else {
        anyhow::bail!("unknown backup type: {name} (expected .sql.gz or .tar.gz)");
    }
    Ok(())
}
