// SPDX-License-Identifier: AGPL-3.0-or-later
//! Client MQTT contre un vrai broker : le Mosquitto de la stack de dev (`make dev-up`, port 11883, `MQTT_TEST_PORT` pour en
//! changer) pour les echanges, et un **Mosquitto jetable** (conteneur nomme, supprime a la fin) pour les coupures. Le broker
//! partage n'est jamais arrete. Chaque test a ses topics (prefixe unique) et ses identifiants de client.

use std::collections::{BTreeMap, BTreeSet};
use std::net::TcpStream;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

use dominet_mqtt::{
    Incoming, MqttClient, MqttSettings, Payload, PublishError, Started, Status, start,
};
use serde_json::json;
use tokio::sync::mpsc;

static COUNTER: AtomicU32 = AtomicU32::new(0);

fn unique(what: &str) -> String {
    let n = COUNTER.fetch_add(1, Ordering::SeqCst);
    let ns = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .subsec_nanos();
    format!("{what}-{}-{n}-{ns}", std::process::id())
}

fn dev_port() -> u16 {
    std::env::var("MQTT_TEST_PORT")
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or(11883)
}

fn client_at(port: u16, what: &str) -> Started {
    start(&MqttSettings::new("127.0.0.1", port, &unique(what)))
}

fn subs(items: &[(&str, u8)]) -> BTreeMap<String, u8> {
    items.iter().map(|(f, q)| ((*f).to_string(), *q)).collect()
}

async fn wait_status(c: &MqttClient, want: Status, secs: u64) -> bool {
    let mut rx = c.watch_status();
    tokio::time::timeout(Duration::from_secs(secs), async {
        loop {
            if *rx.borrow_and_update() == want {
                return;
            }
            if rx.changed().await.is_err() {
                return;
            }
        }
    })
    .await
    .is_ok()
        && c.status() == want
}

async fn recv(rx: &mut mpsc::Receiver<Incoming>, secs: u64) -> Option<Incoming> {
    tokio::time::timeout(Duration::from_secs(secs), rx.recv())
        .await
        .ok()
        .flatten()
}

async fn expect_nothing(rx: &mut mpsc::Receiver<Incoming>) {
    assert!(recv(rx, 1).await.is_none(), "message inattendu");
}

// ------------------------------------------------------------------ Mosquitto jetable

struct Broker {
    name: String,
    port: u16,
    dir: std::path::PathBuf,
}

fn docker(args: &[&str]) -> bool {
    Command::new("docker")
        .args(args)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

impl Broker {
    /// Conteneur nomme sur un port libre, avec persistance (les sessions et messages QoS 1 survivent a un arret propre).
    fn new() -> Self {
        let port = std::net::TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        let name = unique("dominet-test-mosquitto");
        let dir = std::path::PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(&name);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("mosquitto.conf"),
            "listener 1883\nallow_anonymous true\npersistence true\npersistence_location /mosquitto/data/\nautosave_interval 1\n",
        )
        .unwrap();
        let mount = format!(
            "{}:/mosquitto/config/mosquitto.conf:ro",
            dir.join("mosquitto.conf").display()
        );
        let publish = format!("127.0.0.1:{port}:1883");
        assert!(
            docker(&[
                "run",
                "-d",
                "--name",
                &name,
                "-p",
                &publish,
                "-v",
                &mount,
                "eclipse-mosquitto:2.0"
            ]),
            "docker run"
        );
        let b = Self { name, port, dir };
        b.wait_listening(true);
        b
    }

    fn wait_listening(&self, up: bool) {
        let end = Instant::now() + Duration::from_secs(20);
        while Instant::now() < end {
            if TcpStream::connect(("127.0.0.1", self.port)).is_ok() == up {
                std::thread::sleep(Duration::from_millis(200));
                return;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        panic!(
            "le broker jetable n'est pas {}",
            if up { "pret" } else { "arrete" }
        );
    }

    fn stop(&self) {
        assert!(docker(&["stop", "-t", "3", &self.name]), "docker stop");
        self.wait_listening(false);
    }

    fn resume(&self) {
        assert!(docker(&["start", &self.name]), "docker start");
        self.wait_listening(true);
    }
}

impl Drop for Broker {
    fn drop(&mut self) {
        docker(&["rm", "-f", &self.name]);
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// Le numero d'ordre porte par un message (un entier JSON, ou du texte).
fn sequence_of(payload: &Payload) -> Option<u32> {
    match payload {
        Payload::Json(v) => v.as_u64().and_then(|n| u32::try_from(n).ok()),
        Payload::Text(t) => t.parse().ok(),
    }
}

// ------------------------------------------------------------------ broker de dev

#[tokio::test]
async fn messages_round_trip_decoded_with_topic_and_qos() {
    let p = unique("rt");
    let mut sub = client_at(dev_port(), "sub");
    let publ = client_at(dev_port(), "pub");
    sub.client
        .set_subscriptions(subs(&[(&format!("{p}/#"), 1)]));
    assert!(
        wait_status(&sub.client, Status::Connected, 10).await
            && wait_status(&publ.client, Status::Connected, 10).await
    );
    // Laisse le temps au SUBACK avant de publier.
    tokio::time::sleep(Duration::from_millis(300)).await;

    publ.client
        .publish(
            &format!("{p}/salon/lampe"),
            r#"{"state":"ON","brightness":180}"#,
            1,
            false,
        )
        .unwrap();
    publ.client
        .publish(&format!("{p}/texte"), "online", 1, false)
        .unwrap();
    publ.client
        .publish(&format!("{p}/nombre"), "42", 0, false)
        .unwrap();
    let mut got: BTreeMap<String, Incoming> = BTreeMap::new();
    for _ in 0..3 {
        let m = recv(&mut sub.messages, 5).await.expect("message");
        got.insert(m.topic.clone(), m);
    }
    let lamp = &got[&format!("{p}/salon/lampe")];
    assert_eq!(
        lamp.payload,
        Payload::Json(json!({"state": "ON", "brightness": 180}))
    );
    assert!(!lamp.retain && lamp.qos == 1);
    assert_eq!(
        got[&format!("{p}/texte")].payload,
        Payload::Text("online".into())
    );
    assert_eq!(
        got[&format!("{p}/nombre")].payload,
        Payload::Json(json!(42))
    );
    sub.client.shutdown();
    publ.client.shutdown();
}

#[tokio::test]
async fn retained_messages_arrive_on_a_new_subscription_and_can_be_cleared() {
    let p = unique("ret");
    let publ = client_at(dev_port(), "pub");
    assert!(wait_status(&publ.client, Status::Connected, 10).await);
    publ.client
        .publish(&format!("{p}/info"), r#"{"permit_join":false}"#, 1, true)
        .unwrap();
    tokio::time::sleep(Duration::from_millis(300)).await;

    let mut sub = client_at(dev_port(), "sub");
    sub.client
        .set_subscriptions(subs(&[(&format!("{p}/#"), 1)]));
    let m = recv(&mut sub.messages, 10).await.expect("retenu");
    assert!(m.retain);
    assert_eq!(m.payload.as_object().unwrap()["permit_join"], false);
    // Un message retenu vide l'efface (le broker partage n'en garde pas).
    publ.client
        .publish(&format!("{p}/info"), Vec::<u8>::new(), 1, true)
        .unwrap();
    tokio::time::sleep(Duration::from_millis(300)).await;
    sub.client.shutdown();
    publ.client.shutdown();
}

#[tokio::test]
async fn subscriptions_change_while_connected() {
    let p = unique("chg");
    let mut sub = client_at(dev_port(), "sub");
    let publ = client_at(dev_port(), "pub");
    assert!(
        wait_status(&sub.client, Status::Connected, 10).await
            && wait_status(&publ.client, Status::Connected, 10).await
    );
    let (a, b) = (format!("{p}/a"), format!("{p}/b"));
    sub.client.set_subscriptions(subs(&[(&a, 1)]));
    tokio::time::sleep(Duration::from_millis(400)).await;
    publ.client.publish(&a, "1", 1, false).unwrap();
    publ.client.publish(&b, "1", 1, false).unwrap();
    assert_eq!(recv(&mut sub.messages, 5).await.unwrap().topic, a);
    expect_nothing(&mut sub.messages).await;

    // On bascule de a vers b, a chaud.
    sub.client.set_subscriptions(subs(&[(&b, 1)]));
    tokio::time::sleep(Duration::from_millis(400)).await;
    publ.client.publish(&a, "2", 1, false).unwrap();
    publ.client.publish(&b, "2", 1, false).unwrap();
    assert_eq!(recv(&mut sub.messages, 5).await.unwrap().topic, b);
    expect_nothing(&mut sub.messages).await;

    // Plus aucun abonnement : plus aucun message.
    sub.client.set_subscriptions(BTreeMap::new());
    tokio::time::sleep(Duration::from_millis(400)).await;
    publ.client.publish(&b, "3", 1, false).unwrap();
    expect_nothing(&mut sub.messages).await;
    sub.client.shutdown();
    publ.client.shutdown();
}

#[tokio::test]
async fn shutdown_ends_the_task_and_refuses_further_publications() {
    let started = client_at(dev_port(), "stop");
    assert!(wait_status(&started.client, Status::Connected, 10).await);
    started.client.shutdown();
    tokio::time::timeout(Duration::from_secs(5), started.task)
        .await
        .expect("arret de la tache")
        .unwrap();
    assert_eq!(started.client.status(), Status::Disconnected);
    assert_eq!(
        started.client.publish("a/b", "x", 1, false),
        Err(PublishError::NotConnected)
    );
}

// ------------------------------------------------------------------ broker jetable : coupures

#[tokio::test]
async fn a_broker_that_comes_up_late_is_joined_with_the_subscriptions_set_before() {
    let broker_port = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let mut sub = client_at(broker_port, "late-sub");
    let p = unique("late");
    sub.client
        .set_subscriptions(subs(&[(&format!("{p}/#"), 1)]));
    tokio::time::sleep(Duration::from_millis(600)).await;
    assert_ne!(sub.client.status(), Status::Connected);
    // Le broker n'existe pas encore : le conteneur reprend ce port.
    let name = unique("dominet-test-mosquitto");
    let dir = std::path::PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(&name);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("m.conf"), "listener 1883\nallow_anonymous true\n").unwrap();
    let mount = format!(
        "{}:/mosquitto/config/mosquitto.conf:ro",
        dir.join("m.conf").display()
    );
    let publish = format!("127.0.0.1:{broker_port}:1883");
    assert!(docker(&[
        "run",
        "-d",
        "--name",
        &name,
        "-p",
        &publish,
        "-v",
        &mount,
        "eclipse-mosquitto:2.0"
    ]));
    let ok = wait_status(&sub.client, Status::Connected, 30).await;
    let publ = client_at(broker_port, "late-pub");
    let delivered = if ok {
        assert!(wait_status(&publ.client, Status::Connected, 15).await);
        tokio::time::sleep(Duration::from_millis(400)).await;
        publ.client
            .publish(&format!("{p}/x"), "hello", 1, false)
            .unwrap();
        recv(&mut sub.messages, 5).await
    } else {
        None
    };
    docker(&["rm", "-f", &name]);
    let _ = std::fs::remove_dir_all(&dir);
    sub.client.shutdown();
    publ.client.shutdown();
    assert!(ok, "le client doit rejoindre le broker apparu apres lui");
    assert_eq!(
        delivered.expect("message").payload,
        Payload::Text("hello".into())
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn no_acknowledged_message_is_lost_when_the_broker_goes_down_and_comes_back() {
    let broker = Broker::new();
    let p = unique("cut");
    let mut sub = client_at(broker.port, "cut-sub");
    let publ = client_at(broker.port, "cut-pub");
    sub.client
        .set_subscriptions(subs(&[(&format!("{p}/#"), 1)]));
    assert!(
        wait_status(&sub.client, Status::Connected, 15).await
            && wait_status(&publ.client, Status::Connected, 15).await
    );
    tokio::time::sleep(Duration::from_millis(500)).await;

    // Observateur d'etat : le lien de l'abonne doit passer par « coupe » puis revenir.
    let seen_down = {
        let mut rx = sub.client.watch_status();
        tokio::spawn(async move {
            let mut down = false;
            let mut back = false;
            while rx.changed().await.is_ok() {
                match *rx.borrow_and_update() {
                    Status::Disconnected => down = true,
                    Status::Connected if down => {
                        back = true;
                        break;
                    }
                    _ => {}
                }
            }
            (down, back)
        })
    };

    // 100 messages/s pendant ~9 s ; le broker s'arrete 2 s apres le debut, pendant 3 s.
    let total = 900u32;
    let mut accepted: BTreeSet<u32> = BTreeSet::new();
    let mut refused = 0u32;
    let mut tick = tokio::time::interval(Duration::from_millis(10));
    // Les arrets / reprises du broker bloquent cette boucle : sans cela, les ticks manques partiraient en rafale juste apres,
    // avant que le client n'ait pu se reconnecter (delai de reprise jusqu'a 5 s) et tout serait refuse sur une machine rapide.
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    for seq in 1..=total {
        tick.tick().await;
        if seq == 200 {
            let b = &broker;
            tokio::task::block_in_place(|| b.stop());
        }
        if seq == 500 {
            let b = &broker;
            tokio::task::block_in_place(|| b.resume());
            // La reprise est le moment ou l'on cesse de refuser : on attend la reconnexion de l'editeur au lieu de la parier.
            assert!(
                wait_status(&publ.client, Status::Connected, 15).await,
                "l'editeur doit se reconnecter au broker revenu"
            );
            tick.reset();
        }
        match publ
            .client
            .publish(&format!("{p}/seq"), seq.to_string(), 1, false)
        {
            Ok(()) => {
                accepted.insert(seq);
            }
            Err(PublishError::NotConnected) => refused += 1,
            Err(e) => panic!("erreur inattendue : {e}"),
        }
    }

    // Tout message accepte par l'API doit finir par arriver (au moins une fois).
    let mut got: BTreeSet<u32> = BTreeSet::new();
    let deadline = Instant::now() + Duration::from_secs(20);
    while Instant::now() < deadline && !accepted.is_subset(&got) {
        if let Some(m) = recv(&mut sub.messages, 2).await
            && let Some(n) = sequence_of(&m.payload)
        {
            got.insert(n);
        }
    }
    let missing: Vec<_> = accepted.difference(&got).copied().collect();
    let (down, back) = tokio::time::timeout(Duration::from_secs(10), seen_down)
        .await
        .expect("observateur")
        .unwrap();
    sub.client.shutdown();
    publ.client.shutdown();
    assert!(
        missing.is_empty(),
        "messages acceptes mais jamais recus : {missing:?}"
    );
    assert!(
        down && back,
        "l'abonne doit voir la coupure puis la reprise"
    );
    assert!(
        refused > 0,
        "pendant la coupure les publications doivent etre refusees tout de suite"
    );
    assert!(
        accepted.len() > 400 && accepted.iter().any(|n| *n > 500),
        "la plupart des messages passent, y compris apres la reprise : {} acceptes",
        accepted.len()
    );
}
