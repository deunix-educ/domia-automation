// SPDX-License-Identifier: AGPL-3.0-or-later
//! `dominet doctor` : diagnostic en lecture seule, vu depuis l'application (base, migrations, administrateur, cles, MQTT,
//! ReductStore, e-mail, sauvegardes). Chaque constat dit quoi faire. Ne modifie rien, n'affiche aucun secret.
//! La partie « machine » (conteneurs, ports, certificat, disque) est dans `tools/prod.sh doctor`, qui appelle celle-ci.

use std::net::{TcpStream, ToSocketAddrs};
use std::time::Duration;

use sqlx::postgres::PgPoolOptions;

use crate::config::Config;
use crate::secrets::MasterKey;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Level {
    Ok,
    Warning,
    Blocking,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finding {
    pub level: Level,
    pub text: String,
}

impl Finding {
    fn new(level: Level, text: impl Into<String>) -> Self {
        Self {
            level,
            text: text.into(),
        }
    }
}

/// Constats qui ne dependent que de la configuration (testables sans base ni reseau).
#[must_use]
pub fn config_findings(config: &Config) -> Vec<Finding> {
    use Level::{Blocking, Ok, Warning};
    let mut out = Vec::new();
    match &config.master_key {
        None => out.push(Finding::new(
            Warning,
            "DOMINET_MASTER_KEY is empty: camera passwords and other secrets cannot be stored. \
             Fix: `make prod-up` generates one if it is missing, then copy it off the machine.",
        )),
        Some(k) if MasterKey::parse(k.expose()).is_err() => out.push(Finding::new(
            Blocking,
            "DOMINET_MASTER_KEY is not a valid key (32 bytes in base64). \
             Fix: restore the original value; a new key cannot read secrets stored with the old one.",
        )),
        Some(_) => out.push(Finding::new(Ok, "master key present and valid")),
    }
    if config.allowed_hosts.is_empty() {
        out.push(Finding::new(
            Warning,
            "ALLOWED_HOSTS is empty: set the address you type in the browser (LAN_DOMAIN in .env).",
        ));
    } else {
        out.push(Finding::new(
            Ok,
            format!("allowed hosts: {}", config.allowed_hosts.join(", ")),
        ));
    }
    if config.mqtt.is_none() {
        out.push(Finding::new(
            Warning,
            "MQTT is not configured (MQTT_HOST): no device data will arrive.",
        ));
    }
    if config.reductstore.is_none() {
        out.push(Finding::new(
            Warning,
            "ReductStore is not configured (REDUCTSTORE_URL): no history and no video.",
        ));
    }
    if config.email.is_none() {
        out.push(Finding::new(
            Warning,
            "e-mail is not configured (EMAIL_HOST): alerts will not be sent by e-mail.",
        ));
    }
    if config.backup_dir.is_none() {
        out.push(Finding::new(
            Warning,
            "BACKUP_DIR is not set: no scheduled backup.",
        ));
    }
    out
}

fn tcp_reachable(host: &str, port: u16) -> bool {
    (host, port)
        .to_socket_addrs()
        .ok()
        .and_then(|mut a| a.next())
        .is_some_and(|sock| TcpStream::connect_timeout(&sock, Duration::from_secs(3)).is_ok())
}

fn url_host_port(url: &str) -> Option<(String, u16)> {
    let (scheme, rest) = url.split_once("://")?;
    let authority = rest.split('/').next()?;
    let authority = authority.rsplit('@').next()?;
    let default = if scheme == "https" { 443 } else { 80 };
    match authority.rsplit_once(':') {
        Some((h, p)) => Some((h.to_string(), p.parse().ok()?)),
        None => Some((authority.to_string(), default)),
    }
}

async fn database_findings(config: &Config) -> Vec<Finding> {
    use Level::{Blocking, Ok, Warning};
    let connecting = tokio::time::timeout(
        Duration::from_secs(5),
        PgPoolOptions::new()
            .max_connections(1)
            .connect(config.database_url.expose()),
    )
    .await;
    let Result::Ok(Result::Ok(pool)) = connecting else {
        return vec![Finding::new(
            Blocking,
            "the database does not answer. Fix: `make prod-ps` (is postgres healthy?), then `make prod-logs S=postgres N=50`.",
        )];
    };
    let mut out = vec![Finding::new(Ok, "database reachable")];
    let total = dominet_store::MIGRATOR.iter().count();
    let applied: i64 = sqlx::query_scalar("SELECT count(*) FROM _sqlx_migrations WHERE success")
        .fetch_one(&pool)
        .await
        .unwrap_or(0);
    let applied = usize::try_from(applied).unwrap_or(0);
    if applied >= total {
        out.push(Finding::new(
            Ok,
            format!("schema up to date ({total} migrations)"),
        ));
    } else {
        out.push(Finding::new(
            Blocking,
            format!("{applied} of {total} migrations applied. Fix: `make prod-migrate`."),
        ));
    }
    let admins: i64 = sqlx::query_scalar("SELECT count(*) FROM users WHERE role = 'admin'")
        .fetch_one(&pool)
        .await
        .unwrap_or(0);
    if admins > 0 {
        out.push(Finding::new(
            Ok,
            format!("{admins} administrator account(s)"),
        ));
    } else {
        out.push(Finding::new(
            Warning,
            "no administrator account. Fix: `make prod-superuser`.",
        ));
    }
    out
}

/// Tous les constats, dans l'ordre d'affichage.
pub async fn findings(config: &Config) -> Vec<Finding> {
    use Level::{Blocking, Ok};
    let mut out = config_findings(config);
    out.extend(database_findings(config).await);
    if let Some(m) = &config.mqtt {
        let (host, port) = (m.host.clone(), m.port);
        let up = tokio::task::spawn_blocking(move || tcp_reachable(&host, port))
            .await
            .unwrap_or(false);
        out.push(if up {
            Finding::new(Ok, format!("MQTT broker reachable ({}:{})", m.host, m.port))
        } else {
            Finding::new(
                Blocking,
                format!(
                    "the MQTT broker {}:{} does not answer. Fix: `make prod-ps`, then `make prod-logs S=mosquitto N=50`.",
                    m.host, m.port
                ),
            )
        });
    }
    if let Some(r) = &config.reductstore {
        let target = url_host_port(&r.url);
        let up = match target {
            Some((h, p)) => tokio::task::spawn_blocking(move || tcp_reachable(&h, p))
                .await
                .unwrap_or(false),
            None => false,
        };
        out.push(if up {
            Finding::new(Ok, "ReductStore reachable")
        } else {
            Finding::new(
                Blocking,
                "ReductStore does not answer. Fix: `make prod-ps`, then `make prod-logs S=reductstore N=50`.",
            )
        });
    }
    out
}

/// `dominet doctor` : affiche les constats et retourne le nombre de points bloquants.
pub async fn run(config: &Config) -> usize {
    let all = findings(config).await;
    for f in &all {
        let label = match f.level {
            Level::Ok => "OK",
            Level::Warning => "WARNING",
            Level::Blocking => "BLOCKING",
        };
        println!("  {label:<9} {}", f.text);
    }
    let blocking = all.iter().filter(|f| f.level == Level::Blocking).count();
    let warnings = all.iter().filter(|f| f.level == Level::Warning).count();
    println!("\napplication: {blocking} blocking, {warnings} warning(s).");
    blocking
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(env: &[(&str, &str)]) -> Config {
        let map: std::collections::HashMap<String, String> = env
            .iter()
            .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
            .collect();
        Config::from_lookup(|k| map.get(k).cloned()).expect("valid configuration")
    }

    fn has(findings: &[Finding], level: Level, fragment: &str) -> bool {
        findings
            .iter()
            .any(|f| f.level == level && f.text.contains(fragment))
    }

    #[test]
    fn a_minimal_configuration_gets_warnings_not_blockers() {
        let f = config_findings(&config(&[("DATABASE_URL", "postgres://u:p@h/d")]));
        assert!(has(&f, Level::Warning, "DOMINET_MASTER_KEY is empty"));
        assert!(has(&f, Level::Warning, "MQTT is not configured"));
        assert!(has(&f, Level::Warning, "ReductStore is not configured"));
        assert!(f.iter().all(|x| x.level != Level::Blocking));
    }

    #[test]
    fn a_generated_master_key_is_accepted() {
        let key = MasterKey::generate();
        let f = config_findings(&config(&[
            ("DATABASE_URL", "postgres://u:p@h/d"),
            ("DOMINET_MASTER_KEY", key.as_str()),
        ]));
        assert!(has(&f, Level::Ok, "master key present and valid"));
    }

    #[test]
    fn findings_never_contain_secrets() {
        let f = config_findings(&config(&[
            ("DATABASE_URL", "postgres://user:topsecretpw@h/d"),
            ("DOMINET_MASTER_KEY", MasterKey::generate().as_str()),
        ]));
        assert!(f.iter().all(|x| !x.text.contains("topsecretpw")));
    }

    #[test]
    fn url_host_port_reads_common_forms() {
        assert_eq!(
            url_host_port("http://reductstore:8383"),
            Some(("reductstore".into(), 8383))
        );
        assert_eq!(
            url_host_port("https://a.example/x"),
            Some(("a.example".into(), 443))
        );
        assert_eq!(url_host_port("not a url"), None);
    }
}
