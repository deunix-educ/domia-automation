// SPDX-License-Identifier: AGPL-3.0-or-later
//! Journal de l'application en memoire, consultable dans l'administration (`/admin/logs`).
//!
//! Une couche `tracing` ([`LogLayer`]) copie chaque evenement qui passe le filtre `RUST_LOG` dans un tampon circulaire borne
//! ([`CAPACITY`] lignes) : le plus ancien est ecarte quand il est plein. Le tampon ne couvre que **ce processus, depuis son demarrage** ;
//! l'historique complet reste dans les journaux de Docker. Les autres services (nginx, Mosquitto...) n'y figurent pas.

use std::collections::{BTreeSet, VecDeque};
use std::fmt::Write as _;
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

use tracing::field::{Field, Visit};
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::Layer;
use tracing_subscriber::layer::Context;

/// Nombre maximal de lignes gardees.
pub const CAPACITY: usize = 20_000;
/// Longueur maximale d'un message garde (le reste est coupe).
const MAX_MESSAGE: usize = 2048;

/// Une ligne du journal.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LogEntry {
    /// Instant, en millisecondes depuis 1970.
    pub at_ms: i64,
    /// 1 = ERROR, 2 = WARN, 3 = INFO, 4 = DEBUG, 5 = TRACE.
    pub rank: u8,
    pub target: String,
    pub message: String,
}

impl LogEntry {
    /// Nom du niveau.
    #[must_use]
    pub fn level(&self) -> &'static str {
        match self.rank {
            1 => "ERROR",
            2 => "WARN",
            3 => "INFO",
            4 => "DEBUG",
            _ => "TRACE",
        }
    }
}

/// Rang d'un niveau `tracing`.
fn rank_of(level: Level) -> u8 {
    match level {
        Level::ERROR => 1,
        Level::WARN => 2,
        Level::INFO => 3,
        Level::DEBUG => 4,
        Level::TRACE => 5,
    }
}

/// Rang minimal de severite pour un nom de niveau (`error`, `warn`, `info`, `debug`, `trace`) : les lignes de rang inferieur ou egal sont gardees.
#[must_use]
pub fn rank_of_name(name: &str) -> u8 {
    match name.trim().to_ascii_lowercase().as_str() {
        "error" => 1,
        "warn" | "warning" => 2,
        "debug" => 4,
        "trace" | "all" => 5,
        _ => 3,
    }
}

/// Filtre d'une consultation.
#[derive(Clone, Debug, Default)]
pub struct LogQuery {
    /// Rang maximal garde (voir [`rank_of_name`]) ; 0 vaut « tout ».
    pub max_rank: u8,
    /// Le module (`target`) doit contenir ce texte (sans casse) ; vide : tous.
    pub target: String,
    /// Le message doit contenir ce texte (sans casse) ; vide : tous.
    pub text: String,
    /// Instant minimal, en millisecondes depuis 1970.
    pub since_ms: Option<i64>,
    /// Nombre maximal de lignes renvoyees (les plus recentes).
    pub limit: usize,
}

/// Resultat d'une consultation.
#[derive(Clone, Debug, Default)]
pub struct LogPage {
    /// Les lignes retenues, **les plus recentes d'abord**, au plus `limit`.
    pub entries: Vec<LogEntry>,
    /// Combien de lignes correspondent au filtre (avant la limite).
    pub matched: usize,
    /// Combien de lignes sont en memoire.
    pub total: usize,
    /// Les modules vus (pour l'aide a la saisie), tries.
    pub targets: Vec<String>,
}

/// Tampon circulaire.
#[derive(Debug)]
pub struct LogBuffer {
    entries: Mutex<VecDeque<LogEntry>>,
    capacity: usize,
}

impl LogBuffer {
    #[must_use]
    pub fn new(capacity: usize) -> Self {
        Self {
            entries: Mutex::new(VecDeque::with_capacity(capacity.min(1024))),
            capacity: capacity.max(1),
        }
    }

    /// Ajoute une ligne (la plus ancienne est ecartee si le tampon est plein).
    pub fn push(&self, entry: LogEntry) {
        let mut list = self
            .entries
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if list.len() >= self.capacity {
            list.pop_front();
        }
        list.push_back(entry);
    }

    /// Consulte le tampon.
    #[must_use]
    pub fn query(&self, q: &LogQuery) -> LogPage {
        let list = self
            .entries
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let max_rank = if q.max_rank == 0 { 5 } else { q.max_rank };
        let target = q.target.trim().to_lowercase();
        let text = q.text.trim().to_lowercase();
        let mut targets = BTreeSet::new();
        let mut entries = Vec::new();
        let mut matched = 0usize;
        for e in list.iter().rev() {
            targets.insert(e.target.clone());
            let keep = e.rank <= max_rank
                && q.since_ms.is_none_or(|s| e.at_ms >= s)
                && (target.is_empty() || e.target.to_lowercase().contains(&target))
                && (text.is_empty() || e.message.to_lowercase().contains(&text));
            if keep {
                matched += 1;
                if entries.len() < q.limit {
                    entries.push(e.clone());
                }
            }
        }
        LogPage {
            entries,
            matched,
            total: list.len(),
            targets: targets.into_iter().collect(),
        }
    }
}

/// Le tampon du processus.
#[must_use]
pub fn global() -> &'static LogBuffer {
    static BUFFER: OnceLock<LogBuffer> = OnceLock::new();
    BUFFER.get_or_init(|| LogBuffer::new(CAPACITY))
}

/// Instant courant, en millisecondes depuis 1970.
#[must_use]
pub fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
}

/// Couche `tracing` qui alimente le tampon global.
#[derive(Clone, Copy, Debug, Default)]
pub struct LogLayer;

#[derive(Default)]
struct Collector {
    message: String,
    fields: String,
}

impl Visit for Collector {
    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        if field.name() == "message" {
            let _ = write!(self.message, "{value:?}");
        } else {
            if !self.fields.is_empty() {
                self.fields.push(' ');
            }
            let _ = write!(self.fields, "{}={value:?}", field.name());
        }
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        if field.name() == "message" {
            self.message.push_str(value);
        } else {
            if !self.fields.is_empty() {
                self.fields.push(' ');
            }
            let _ = write!(self.fields, "{}={value}", field.name());
        }
    }
}

impl<S: Subscriber> Layer<S> for LogLayer {
    fn on_event(&self, event: &Event<'_>, _ctx: Context<'_, S>) {
        let mut c = Collector::default();
        event.record(&mut c);
        let mut message = c.message;
        if !c.fields.is_empty() {
            if !message.is_empty() {
                message.push(' ');
            }
            message.push_str(&c.fields);
        }
        if message.len() > MAX_MESSAGE {
            let mut cut = MAX_MESSAGE;
            while !message.is_char_boundary(cut) {
                cut -= 1;
            }
            message.truncate(cut);
            message.push('…');
        }
        let meta = event.metadata();
        global().push(LogEntry {
            at_ms: now_ms(),
            rank: rank_of(*meta.level()),
            target: meta.target().to_string(),
            message,
        });
    }
}

/// `2026-11-13 09:12:05` dans le fuseau donne.
#[must_use]
pub fn format_local(at_ms: i64, tz: chrono_tz::Tz) -> String {
    use chrono::TimeZone;
    chrono::Utc
        .timestamp_millis_opt(at_ms)
        .single()
        .map_or_else(String::new, |t| {
            t.with_timezone(&tz).format("%Y-%m-%d %H:%M:%S").to_string()
        })
}

/// Comme [`format_local`], avec le nom du fuseau (`Europe/Paris`) ; un nom inconnu donne l'heure UTC.
#[must_use]
pub fn format_local_name(at_ms: i64, tz_name: &str) -> String {
    format_local(at_ms, tz_name.parse().unwrap_or(chrono_tz::UTC))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(at_ms: i64, rank: u8, target: &str, message: &str) -> LogEntry {
        LogEntry {
            at_ms,
            rank,
            target: target.into(),
            message: message.into(),
        }
    }

    fn sample() -> LogBuffer {
        let b = LogBuffer::new(4);
        b.push(entry(1_000, 3, "dominet_mqtt::client", "MQTT connected"));
        b.push(entry(
            2_000,
            2,
            "dominet_integrations::run",
            "onduleur injoignable",
        ));
        b.push(entry(
            3_000,
            1,
            "dominet_server::rules",
            "Echec du rechargement",
        ));
        b.push(entry(4_000, 4, "dominet_mqtt::client", "paquet recu"));
        b
    }

    #[test]
    fn the_newest_lines_come_first_and_the_limit_keeps_the_newest() {
        let page = sample().query(&LogQuery {
            limit: 2,
            ..LogQuery::default()
        });
        assert_eq!(page.total, 4);
        assert_eq!(page.matched, 4);
        let times: Vec<i64> = page.entries.iter().map(|e| e.at_ms).collect();
        assert_eq!(times, [4_000, 3_000]);
    }

    #[test]
    fn filters_combine_level_module_text_and_period() {
        let b = sample();
        let warn = b.query(&LogQuery {
            max_rank: rank_of_name("warn"),
            limit: 10,
            ..LogQuery::default()
        });
        assert_eq!(warn.matched, 2, "ERROR et WARN seulement");
        let module = b.query(&LogQuery {
            target: "MQTT".into(),
            limit: 10,
            ..LogQuery::default()
        });
        assert_eq!(module.matched, 2, "module sans casse");
        let text = b.query(&LogQuery {
            text: "injoignable".into(),
            limit: 10,
            ..LogQuery::default()
        });
        assert_eq!(text.entries[0].message, "onduleur injoignable");
        let recent = b.query(&LogQuery {
            since_ms: Some(3_000),
            limit: 10,
            ..LogQuery::default()
        });
        assert_eq!(recent.matched, 2);
        let none = b.query(&LogQuery {
            text: "absent".into(),
            limit: 10,
            ..LogQuery::default()
        });
        assert_eq!((none.matched, none.entries.len(), none.total), (0, 0, 4));
        assert_eq!(
            b.query(&LogQuery {
                limit: 1,
                ..LogQuery::default()
            })
            .targets
            .len(),
            3,
            "modules vus, dedoublonnes"
        );
    }

    #[test]
    fn the_buffer_drops_the_oldest_when_full() {
        let b = LogBuffer::new(2);
        for i in 0..5 {
            b.push(entry(i, 3, "t", &format!("ligne {i}")));
        }
        let page = b.query(&LogQuery {
            limit: 10,
            ..LogQuery::default()
        });
        assert_eq!(page.total, 2);
        assert_eq!(page.entries[0].message, "ligne 4");
        assert_eq!(page.entries[1].message, "ligne 3");
    }

    #[test]
    fn level_names_map_to_ranks_and_times_use_the_zone() {
        assert_eq!(rank_of_name("ERROR"), 1);
        assert_eq!(rank_of_name("warn"), 2);
        assert_eq!(rank_of_name("info"), 3);
        assert_eq!(rank_of_name("inconnu"), 3);
        assert_eq!(rank_of_name("debug"), 4);
        assert_eq!(rank_of_name("all"), 5);
        // 2026-11-13 08:00:00 UTC = 09:00:00 a Paris (heure d'hiver).
        assert_eq!(
            format_local(1_794_556_800_000, chrono_tz::Europe::Paris),
            "2026-11-13 09:00:00"
        );
    }
}
