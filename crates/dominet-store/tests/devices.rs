// SPDX-License-Identifier: AGPL-3.0-or-later
//! Schema (migration 0008) and repositories of devices, groups, labels, history, images, MQTT sources and weather stations.
//! Chaque test a sa base isolee (`make dev-up`).

use chrono::{Duration, TimeZone, Utc};
use dominet_domain::RepoError;
use dominet_domain::dashboard::{DashboardDraft, WidgetDraft, WidgetType};
use dominet_domain::device::{
    DeviceDraft, DeviceGroupDraft, DeviceId, DeviceType, Discovered, LogEntry, MqttSource,
    WeatherStation, known_label,
};
use dominet_domain::location::LocationDraft;
use dominet_domain::repo::{
    DashboardRepo, DeviceFilter, DeviceGroupRepo, DeviceImageRepo, DeviceLabelRepo, DeviceRepo,
    LocationRepo, MqttSourceRepo, StateLogRepo, UserRepo, WeatherRepo, WidgetRepo,
};
use dominet_domain::user::{NewUser, Role, Username};
use dominet_store::PgStore;
use serde_json::json;
use sqlx::PgPool;

fn discovered(name: &str) -> Discovered {
    Discovered {
        friendly_name: name.to_string(),
        ieee: "0x00124b0000000001".into(),
        vendor: "Test".into(),
        model_id: "TH-01".into(),
        description: "Capteur".into(),
        exposes: vec![json!({"property": "temperature", "unit": "°C"})],
    }
}

async fn expect_violation(pool: &PgPool, sql: &'static str) {
    let r = sqlx::query(sql).execute(pool).await;
    assert!(r.is_err(), "accepte a tort : {sql}");
}

// ------------------------------------------------------------------ schema

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn the_device_tables_enforce_their_constraints(pool: PgPool) {
    sqlx::query("INSERT INTO device (friendly_name) VALUES ('a')")
        .execute(&pool)
        .await
        .unwrap();
    for bad in [
        "INSERT INTO device (friendly_name) VALUES ('a')", // unicite
        "INSERT INTO device (friendly_name) VALUES ('')",  // nom vide
        "INSERT INTO device (friendly_name, device_type) VALUES ('b', 'fan')", // type inconnu
        "INSERT INTO device (friendly_name, state) VALUES ('b', '[]')", // the state is an object
        "INSERT INTO device (friendly_name, exposes) VALUES ('b', '{}')", // exposes is an array
        "INSERT INTO device (friendly_name, group_id) VALUES ('b', 999)", // groupe inexistant
        "INSERT INTO device_label (key, name, icon_name) VALUES ('', 'x', 'y')",
        "INSERT INTO mqtt_source (name, topic_pattern, qos) VALUES ('s', 'a/#', 3)",
        "INSERT INTO mqtt_source (name, topic_pattern, auto_device_type) VALUES ('s', 'a/#', 'fan')",
        "INSERT INTO weather_station (name, latitude, longitude) VALUES ('m', 91, 0)",
        "INSERT INTO weather_station (name, latitude, longitude, mqtt_topic_suffix) VALUES ('m', 0, 0, 'a b')",
        "INSERT INTO weather_station (name, latitude, longitude, refresh_minutes) VALUES ('m', 0, 0, 0)",
        "INSERT INTO device_image (device_id, content_type, data) VALUES (1, 'text/html', 'x')",
        "INSERT INTO device_state_log (device_id, key, ts) VALUES (999, 'k', now())",
    ] {
        expect_violation(&pool, bad).await;
    }
    // Default values of the reference implementation.
    let (base, hidden, available): (String, Vec<String>, bool) = sqlx::query_as(
        "SELECT base_topic, hidden_keys, available FROM device WHERE friendly_name = 'a'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        (base.as_str(), hidden.len(), available),
        ("zigbee2mqtt", 0, false)
    );
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn history_rows_are_unique_per_second_and_follow_the_device(pool: PgPool) {
    let id: i64 =
        sqlx::query_scalar("INSERT INTO device (friendly_name) VALUES ('a') RETURNING id")
            .fetch_one(&pool)
            .await
            .unwrap();
    let insert = "INSERT INTO device_state_log (device_id, key, ts, value_num) VALUES ($1, 't', '2026-01-01T10:00:00Z', 1)";
    sqlx::query(insert).bind(id).execute(&pool).await.unwrap();
    assert!(
        sqlx::query(insert).bind(id).execute(&pool).await.is_err(),
        "meme appareil, cle et seconde"
    );
    sqlx::query("DELETE FROM device WHERE id = $1")
        .bind(id)
        .execute(&pool)
        .await
        .unwrap();
    let left: i64 = sqlx::query_scalar("SELECT count(*) FROM device_state_log")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(left, 0, "l'historique suit l'appareil supprime");
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn widgets_reference_devices_with_a_foreign_key_that_nulls_on_delete(pool: PgPool) {
    let store = PgStore::new(pool.clone());
    let user = store
        .users()
        .create(&NewUser {
            username: Username::parse("alice").unwrap(),
            email: "a@example.org".into(),
            first_name: String::new(),
            last_name: String::new(),
            mobile: String::new(),
            role: Role::Admin,
            is_active: true,
            password_hash: "h".into(),
        })
        .await
        .unwrap();
    let board = store
        .dashboards()
        .create(
            user.id,
            &DashboardDraft {
                name: "Salon".into(),
                is_default: false,
            },
        )
        .await
        .unwrap();
    let device = store
        .devices()
        .create(&DeviceDraft::new("Sonde"))
        .await
        .unwrap();
    let mut draft = WidgetDraft::new(WidgetType::Device);
    draft.device_id = Some(device.id.0);
    let widget = store.widgets().create(board.id, &draft).await.unwrap();
    // A reference to a non-existent device is refused (this is what differentiates R3 from R2).
    let mut orphan = WidgetDraft::new(WidgetType::Device);
    orphan.device_id = Some(424_242);
    orphan.rect.col = 6;
    assert!(matches!(
        store.widgets().create(board.id, &orphan).await,
        Err(RepoError::Conflict { .. })
    ));
    // Deleting the device keeps the widget, without a reference.
    store.devices().delete(device.id).await.unwrap();
    let kept = store.widgets().get(widget.id).await.unwrap().unwrap();
    assert_eq!(kept.device_id, None);
}

// ------------------------------------------------------------------ appareils

async fn only(repo: &dominet_store::PgDevices, f: DeviceFilter) -> Vec<DeviceId> {
    repo.list(&f)
        .await
        .unwrap()
        .into_iter()
        .map(|d| d.id)
        .collect()
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn discovery_creates_with_reference_defaults_then_only_refreshes_the_metadata(pool: PgPool) {
    let repo = PgStore::new(pool).devices();
    let (d, created) = repo
        .upsert_discovered(&discovered("Bureau"), "zigbee2mqtt")
        .await
        .unwrap();
    assert!(created);
    assert_eq!(
        (
            d.base_topic.as_str(),
            d.hidden_keys.as_slice(),
            d.device_type,
            d.active,
            d.available
        ),
        (
            "zigbee2mqtt",
            ["update".to_string()].as_slice(),
            DeviceType::Sensor,
            true,
            false
        )
    );
    assert_eq!(
        (d.ieee.as_str(), d.vendor.as_str(), d.exposes.len()),
        ("0x00124b0000000001", "Test", 1)
    );
    // The administrator adapts the device and a state message arrives.
    let mut draft = DeviceDraft::new("Bureau");
    draft.device_type = DeviceType::Climate;
    draft.tracked_keys = vec!["temperature".into()];
    draft.icon = "thermometer".into();
    repo.update(d.id, &draft).await.unwrap();
    repo.apply_state("Bureau", &json!({"temperature": 20}))
        .await
        .unwrap()
        .unwrap();
    // A new discovery touches neither the type, nor the tracking, nor the state; it refreshes the description.
    let mut again = discovered("Bureau");
    again.description = "Capteur v2".into();
    again.exposes = vec![];
    let (e, created) = repo.upsert_discovered(&again, "zigbee2mqtt").await.unwrap();
    assert!(!created);
    assert_eq!(e.id, d.id);
    assert_eq!((e.description.as_str(), e.exposes.len()), ("Capteur v2", 0));
    assert_eq!(
        (e.device_type, e.tracked_keys.as_slice(), e.icon.as_str()),
        (
            DeviceType::Climate,
            ["temperature".to_string()].as_slice(),
            "thermometer"
        )
    );
    assert_eq!(e.state, json!({"temperature": 20}));
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn a_state_message_replaces_the_state_and_only_reaches_active_devices(pool: PgPool) {
    let repo = PgStore::new(pool).devices();
    let (d, _) = repo
        .upsert_discovered(&discovered("Bureau"), "zigbee2mqtt")
        .await
        .unwrap();
    assert!(d.last_seen.is_none());
    let a = repo
        .apply_state("Bureau", &json!({"temperature": 21, "humidity": 40}))
        .await
        .unwrap()
        .unwrap();
    assert!(a.available && a.last_seen.is_some());
    // Replacement, not merge.
    let b = repo
        .apply_state("Bureau", &json!({"temperature": 22}))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(b.state, json!({"temperature": 22}));
    // Inconnu : rien. Inactif : rien.
    assert!(
        repo.apply_state("Unknown", &json!({}))
            .await
            .unwrap()
            .is_none()
    );
    let mut off = DeviceDraft::new("Bureau");
    off.active = false;
    repo.update(d.id, &off).await.unwrap();
    assert!(
        repo.apply_state("Bureau", &json!({"temperature": 99}))
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(
        repo.get(d.id).await.unwrap().unwrap().state,
        json!({"temperature": 22})
    );
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn availability_changes_do_not_touch_last_seen(pool: PgPool) {
    let repo = PgStore::new(pool).devices();
    repo.upsert_discovered(&discovered("Bureau"), "zigbee2mqtt")
        .await
        .unwrap();
    let seen = repo
        .apply_state("Bureau", &json!({"a": 1}))
        .await
        .unwrap()
        .unwrap()
        .last_seen;
    let off = repo.set_available("Bureau", false).await.unwrap().unwrap();
    assert!(!off.available);
    assert_eq!(off.last_seen, seen);
    assert_eq!(off.state, json!({"a": 1}), "l'etat est conserve");
    assert!(repo.set_available("Unknown", true).await.unwrap().is_none());
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn auto_devices_are_created_updated_and_reactivated_without_changing_their_type(
    pool: PgPool,
) {
    let repo = PgStore::new(pool).devices();
    let (d, created) = repo
        .upsert_auto(
            "0x6f09/prod",
            "solarmax",
            DeviceType::Sensor,
            &json!({"pac": 120.5}),
        )
        .await
        .unwrap();
    assert!(created);
    assert_eq!(
        (d.base_topic.as_str(), d.device_type, d.available),
        ("solarmax", DeviceType::Sensor, true)
    );
    // The administrator makes it a switch and deactivates it; the next message reactivates it, without touching the type.
    let mut draft = DeviceDraft::new("0x6f09/prod");
    draft.device_type = DeviceType::Switch;
    draft.base_topic = "solarmax".into();
    draft.active = false;
    repo.update(d.id, &draft).await.unwrap();
    let (e, created) = repo
        .upsert_auto(
            "0x6f09/prod",
            "solarmax",
            DeviceType::Sensor,
            &json!({"pac": 130}),
        )
        .await
        .unwrap();
    assert!(!created);
    assert_eq!(
        (e.id, e.device_type, e.active, e.state),
        (d.id, DeviceType::Switch, true, json!({"pac": 130}))
    );
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn a_topic_marks_the_devices_it_belongs_to_as_seen(pool: PgPool) {
    let repo = PgStore::new(pool).devices();
    repo.upsert_discovered(&discovered("salon/lampe"), "zigbee2mqtt")
        .await
        .unwrap(); // zigbee2mqtt/salon/lampe
    let (agg, _) = repo
        .upsert_auto("x", "linky/consommation", DeviceType::Sensor, &json!({}))
        .await
        .unwrap(); // base_topic == topic
    repo.set_available("salon/lampe", false).await.unwrap();
    let hit = repo
        .touch_by_topic("zigbee2mqtt/salon/lampe")
        .await
        .unwrap();
    assert_eq!(hit.len(), 1);
    assert!(
        repo.find_by_name("salon/lampe")
            .await
            .unwrap()
            .unwrap()
            .available
    );
    assert_eq!(
        repo.touch_by_topic("linky/consommation").await.unwrap(),
        [agg.id]
    );
    assert_eq!(
        repo.touch_by_topic("autre/topic").await.unwrap(),
        Vec::<DeviceId>::new()
    );
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn devices_are_listed_with_filters_and_names_stay_unique(pool: PgPool) {
    let store = PgStore::new(pool);
    let repo = store.devices();
    let loc = store
        .locations()
        .create(&LocationDraft {
            name: "Salon".into(),
            description: String::new(),
            active: true,
        })
        .await
        .unwrap();
    let group = store
        .device_groups()
        .create(&DeviceGroupDraft {
            name: "Lumieres".into(),
            location: Some(loc.id),
            description: String::new(),
            icon: String::new(),
        })
        .await
        .unwrap();
    let mut a = DeviceDraft::new("a");
    a.device_type = DeviceType::Light;
    a.group = Some(group.id);
    a.location = Some(loc.id);
    let mut b = DeviceDraft::new("b");
    b.active = false;
    let (a, b) = (
        repo.create(&a).await.unwrap(),
        repo.create(&b).await.unwrap(),
    );
    assert!(matches!(
        repo.create(&DeviceDraft::new("a")).await,
        Err(RepoError::Conflict { .. })
    ));
    let all = repo.list(&DeviceFilter::default()).await.unwrap();
    assert_eq!(
        all.iter()
            .map(|d| d.friendly_name.as_str())
            .collect::<Vec<_>>(),
        ["a", "b"]
    );
    assert_eq!(
        only(
            &repo,
            DeviceFilter {
                active_only: true,
                ..Default::default()
            }
        )
        .await,
        [a.id]
    );
    assert_eq!(
        only(
            &repo,
            DeviceFilter {
                device_type: Some(DeviceType::Light),
                ..Default::default()
            }
        )
        .await,
        [a.id]
    );
    assert_eq!(
        only(
            &repo,
            DeviceFilter {
                group: Some(group.id),
                ..Default::default()
            }
        )
        .await,
        [a.id]
    );
    assert_eq!(
        only(
            &repo,
            DeviceFilter {
                location: Some(loc.id),
                ..Default::default()
            }
        )
        .await,
        [a.id]
    );
    assert_eq!(
        only(
            &repo,
            DeviceFilter {
                device_type: Some(DeviceType::Cover),
                ..Default::default()
            }
        )
        .await,
        Vec::<DeviceId>::new()
    );
    // Renaming to a taken name is a conflict; deleting a group or a location leaves the device.
    assert!(matches!(
        repo.update(b.id, &DeviceDraft::new("a")).await,
        Err(RepoError::Conflict { .. })
    ));
    store.device_groups().delete(group.id).await.unwrap();
    store.locations().delete(loc.id).await.unwrap();
    let kept = repo.get(a.id).await.unwrap().unwrap();
    assert_eq!((kept.group, kept.location), (None, None));
    assert!(matches!(
        repo.update(DeviceId(999), &DeviceDraft::new("z")).await,
        Err(RepoError::NotFound)
    ));
    assert!(matches!(
        repo.delete(DeviceId(999)).await,
        Err(RepoError::NotFound)
    ));
}

// ------------------------------------------------------------------ libelles, historique, images

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn missing_labels_are_created_once_from_the_state_keys(pool: PgPool) {
    let store = PgStore::new(pool);
    store
        .devices()
        .upsert_auto(
            "a",
            "",
            DeviceType::Sensor,
            &json!({"temperature": 1, "co2_ppm": 400}),
        )
        .await
        .unwrap();
    store
        .devices()
        .upsert_auto(
            "b",
            "",
            DeviceType::Sensor,
            &json!({"temperature": 2, "battery": 90}),
        )
        .await
        .unwrap();
    let labels = store.device_labels();
    let keys = labels.state_keys().await.unwrap();
    assert_eq!(keys, ["battery", "co2_ppm", "temperature"]);
    let drafts: Vec<_> = keys.iter().map(|k| known_label(k)).collect();
    assert_eq!(labels.create_missing(&drafts).await.unwrap(), 3);
    assert_eq!(
        labels.create_missing(&drafts).await.unwrap(),
        0,
        "idempotent"
    );
    let all = labels.list().await.unwrap();
    let temperature = all.iter().find(|l| l.key == "temperature").unwrap();
    assert_eq!(
        (temperature.icon_name.as_str(), temperature.unit.as_str()),
        ("thermometer", "°C")
    );
    assert_eq!(
        all.iter().find(|l| l.key == "co2_ppm").unwrap().icon_name,
        "circle-dot"
    );
    assert!(matches!(
        labels.create(&known_label("battery")).await,
        Err(RepoError::Conflict { .. })
    ));
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn history_is_written_per_second_replaced_on_duplicates_and_purged(pool: PgPool) {
    let store = PgStore::new(pool);
    let (d, _) = store
        .devices()
        .upsert_auto("a", "", DeviceType::Sensor, &json!({}))
        .await
        .unwrap();
    let log = store.state_log();
    let t0 = Utc.with_ymd_and_hms(2026, 1, 1, 10, 0, 0).unwrap();
    let entry = |k: &str, n: Option<f64>, s: &str| LogEntry {
        key: k.into(),
        value_num: n,
        value_str: s.into(),
    };
    assert_eq!(
        log.record(
            d.id,
            t0,
            &[
                entry("temperature", Some(20.0), ""),
                entry("mode", None, "eco"),
                entry("contact", Some(1.0), "True")
            ]
        )
        .await
        .unwrap(),
        3
    );
    assert_eq!(log.record(d.id, t0, &[]).await.unwrap(), 0);
    // A QoS 1 duplicate within the same second replaces the value instead of adding a row.
    log.record(d.id, t0, &[entry("temperature", Some(20.5), "")])
        .await
        .unwrap();
    log.record(
        d.id,
        t0 + Duration::seconds(1),
        &[entry("temperature", Some(21.0), "")],
    )
    .await
    .unwrap();
    log.record(
        d.id,
        t0 + Duration::seconds(2),
        &[entry("temperature", Some(22.0), "")],
    )
    .await
    .unwrap();
    let pts = log.points(d.id, "temperature", t0, 100).await.unwrap();
    assert_eq!(
        pts.iter().map(|p| p.value_num).collect::<Vec<_>>(),
        [Some(20.5), Some(21.0), Some(22.0)]
    );
    assert_eq!(
        log.points(d.id, "temperature", t0 + Duration::seconds(1), 100)
            .await
            .unwrap()
            .len(),
        2
    );
    assert_eq!(
        log.points(d.id, "temperature", t0, 1).await.unwrap().len(),
        1
    );
    let mode = log.points(d.id, "mode", t0, 10).await.unwrap();
    assert_eq!(
        (mode[0].value_num, mode[0].value_str.as_str()),
        (None, "eco")
    );
    let contact = log.points(d.id, "contact", t0, 10).await.unwrap();
    assert_eq!(
        (contact[0].value_num, contact[0].value_str.as_str()),
        (Some(1.0), "True")
    );
    // Purge: only the strictly older rows go.
    assert_eq!(
        log.purge_before(t0 + Duration::seconds(1)).await.unwrap(),
        3
    );
    assert_eq!(
        log.points(d.id, "temperature", t0, 100)
            .await
            .unwrap()
            .len(),
        2
    );
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn images_are_stored_replaced_and_follow_the_device(pool: PgPool) {
    let store = PgStore::new(pool);
    let images = store.device_images();
    let d = store
        .devices()
        .create(&DeviceDraft::new("a"))
        .await
        .unwrap();
    assert!(images.get(d.id).await.unwrap().is_none());
    assert_eq!(images.with_images().await.unwrap(), vec![]);
    images.put(d.id, "image/jpeg", &[1, 2, 3]).await.unwrap();
    images.put(d.id, "image/png", &[4, 5]).await.unwrap();
    let listed = images.with_images().await.unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].0, d.id);
    let got = images.get(d.id).await.unwrap().unwrap();
    assert_eq!(
        (got.content_type.as_str(), got.data.as_slice()),
        ("image/png", [4u8, 5].as_slice())
    );
    assert!(matches!(
        images.put(DeviceId(999), "image/png", &[1]).await,
        Err(RepoError::NotFound)
    ));
    images.delete(d.id).await.unwrap();
    assert!(images.get(d.id).await.unwrap().is_none());
    images.put(d.id, "image/gif", &[9]).await.unwrap();
    store.devices().delete(d.id).await.unwrap();
    assert!(
        images.get(d.id).await.unwrap().is_none(),
        "l'image suit l'appareil"
    );
}

// ------------------------------------------------------------------ sources and stations

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn mqtt_sources_round_trip_and_only_active_ones_are_subscribed(pool: PgPool) {
    let repo = PgStore::new(pool).mqtt_sources();
    let mut linky = MqttSource::draft("Linky", "linky/#");
    linky.qos = 1;
    linky.auto_device = true;
    linky.auto_device_type = None;
    linky.base_topic = "linky".into();
    linky.handler_code = "print(payload)".into();
    let mut off = MqttSource::draft("Ancienne", "old/+");
    off.active = false;
    let linky = repo.create(&linky).await.unwrap();
    repo.create(&off).await.unwrap();
    assert_eq!(
        (
            linky.qos,
            linky.auto_device_type,
            linky.handler_code.as_str()
        ),
        (1, None, "print(payload)")
    );
    let active = repo.list_active().await.unwrap();
    assert_eq!(
        active.iter().map(|s| s.name.as_str()).collect::<Vec<_>>(),
        ["Linky"]
    );
    assert_eq!(repo.list().await.unwrap().len(), 2);
    let mut edit = MqttSource::draft("Linky", "linky/+/conso");
    edit.auto_device_type = Some(DeviceType::Switch);
    let edited = repo.update(linky.id, &edit).await.unwrap();
    assert_eq!(
        (edited.topic_pattern.as_str(), edited.auto_device_type),
        ("linky/+/conso", Some(DeviceType::Switch))
    );
    repo.delete(linky.id).await.unwrap();
    assert!(repo.get(linky.id).await.unwrap().is_none());
    assert!(matches!(
        repo.delete(linky.id).await,
        Err(RepoError::NotFound)
    ));
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn weather_stations_round_trip_and_only_enabled_ones_are_polled(pool: PgPool) {
    let repo = PgStore::new(pool).weather();
    let home = repo
        .create(&WeatherStation::draft("Maison", 48.8566, 2.3522))
        .await
        .unwrap();
    let mut off = WeatherStation::draft("Chalet", 45.9, 6.1);
    off.enabled = false;
    off.altitude = 1200;
    off.mqtt_topic_suffix = "chalet".into();
    let chalet = repo.create(&off).await.unwrap();
    assert_eq!(
        (
            home.timezone.as_str(),
            home.refresh_minutes,
            home.mqtt_topic().as_str()
        ),
        ("Europe/Paris", 10, "meteo/meteo_locale")
    );
    assert_eq!(
        (chalet.altitude, chalet.mqtt_topic().as_str()),
        (1200, "meteo/chalet")
    );
    assert_eq!(
        repo.list_enabled()
            .await
            .unwrap()
            .iter()
            .map(|s| s.name.as_str())
            .collect::<Vec<_>>(),
        ["Maison"]
    );
    let mut edit = chalet.clone();
    edit.enabled = true;
    edit.refresh_minutes = 30;
    assert_eq!(
        repo.update(chalet.id, &edit).await.unwrap().refresh_minutes,
        30
    );
    assert_eq!(repo.list_enabled().await.unwrap().len(), 2);
    repo.delete(home.id).await.unwrap();
    assert!(repo.get(home.id).await.unwrap().is_none());
}
