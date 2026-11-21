// SPDX-License-Identifier: AGPL-3.0-or-later
//! Sauvegarde et restauration (R7 lot 4, ADR 0054) : la base PostgreSQL (`pg_dump`, SQL compresse) et les donnees de ReductStore (archive
//! tar.gz du volume monte en lecture seule). Chaque fichier a sa somme SHA-256 a cote ; une restauration la verifie d'abord.
//!
//! Dossier : `<BACKUP_DIR>/postgres/dominet_<AAAAMMJJ_HHMMSS>.sql.gz` et `<BACKUP_DIR>/reductstore/reductstore_<...>.tar.gz`
//! (noms stables, horodates). Le mot de passe de la base ne passe jamais par la ligne de
//! commande des clients PostgreSQL : il est transmis par l'environnement du processus fils (`PGPASSWORD`).

use std::fs::{self, File};
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, SystemTime};

use chrono::{DateTime, TimeZone, Utc};
use flate2::Compression;
use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use sha2::{Digest, Sha256};

use crate::AppState;
use crate::config::Config;

const POSTGRES_DIR: &str = "postgres";
const REDUCTSTORE_DIR: &str = "reductstore";

#[derive(Debug, thiserror::Error)]
pub enum BackupError {
    #[error("BACKUP_DIR n'est pas renseigne : aucune destination de sauvegarde")]
    NoDestination,
    #[error("adresse de la base inexploitable : {0}")]
    BadDatabaseUrl(String),
    #[error("{what} : {source}")]
    Io {
        what: String,
        #[source]
        source: std::io::Error,
    },
    #[error("{program} a echoue ({status}) : {stderr}")]
    Client {
        program: String,
        status: String,
        stderr: String,
    },
    #[error("somme de controle incorrecte pour {0} : le fichier est abime ou a ete modifie")]
    Corrupt(PathBuf),
    #[error("{0}")]
    Refused(String),
}

fn io(what: impl Into<String>) -> impl FnOnce(std::io::Error) -> BackupError {
    let what = what.into();
    move |source| BackupError::Io { what, source }
}

/// Connexion a la base, extraite de `DATABASE_URL`, transmise aux clients PostgreSQL par l'environnement.
#[derive(Clone)]
struct PgTarget {
    host: String,
    port: u16,
    user: String,
    password: String,
    database: String,
    sslmode: Option<String>,
}

impl std::fmt::Debug for PgTarget {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PgTarget")
            .field("host", &self.host)
            .field("port", &self.port)
            .field("user", &self.user)
            .field("password", &"***")
            .field("database", &self.database)
            .finish_non_exhaustive()
    }
}

fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%'
            && i + 2 < b.len()
            && let (Some(h), Some(l)) = (
                (b[i + 1] as char).to_digit(16),
                (b[i + 2] as char).to_digit(16),
            )
        {
            out.push(u8::try_from(h * 16 + l).unwrap_or(b'?'));
            i += 3;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

impl PgTarget {
    fn parse(url: &str) -> Result<Self, BackupError> {
        let u = url::Url::parse(url).map_err(|e| BackupError::BadDatabaseUrl(e.to_string()))?;
        if !matches!(u.scheme(), "postgres" | "postgresql") {
            return Err(BackupError::BadDatabaseUrl(
                "postgres:// scheme expected".into(),
            ));
        }
        let database = u.path().trim_start_matches('/');
        if database.is_empty() {
            return Err(BackupError::BadDatabaseUrl("database name missing".into()));
        }
        Ok(Self {
            host: u.host_str().unwrap_or("localhost").to_string(),
            port: u.port().unwrap_or(5432),
            user: percent_decode(u.username()),
            password: percent_decode(u.password().unwrap_or_default()),
            database: percent_decode(database),
            sslmode: u
                .query_pairs()
                .find(|(k, _)| k == "sslmode")
                .map(|(_, v)| v.into_owned()),
        })
    }

    fn apply(&self, cmd: &mut Command) {
        cmd.env("PGHOST", &self.host)
            .env("PGPORT", self.port.to_string())
            .env("PGUSER", &self.user)
            .env("PGDATABASE", &self.database);
        if !self.password.is_empty() {
            cmd.env("PGPASSWORD", &self.password);
        }
        if let Some(mode) = &self.sslmode {
            cmd.env("PGSSLMODE", mode);
        }
    }
}

/// Une sauvegarde ecrite.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Artifact {
    pub path: PathBuf,
    pub bytes: u64,
}

/// Resultat d'un passage.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackupReport {
    pub postgres: Artifact,
    /// `None` si `REDUCTSTORE_BACKUP_SRC` n'est pas renseigne ou introuvable.
    pub reductstore: Option<Artifact>,
    pub pruned: usize,
}

/// Service de sauvegarde (construit depuis la configuration).
#[derive(Debug, Clone)]
pub struct BackupService {
    dir: Option<PathBuf>,
    retention: Duration,
    reductstore_src: Option<PathBuf>,
    pg_dump: String,
    psql: String,
    db: Result<PgTarget, String>,
}

/// Horodatage des noms de fichiers (UTC, comme la reference).
fn stamp(now: DateTime<Utc>) -> String {
    now.format("%Y%m%d_%H%M%S").to_string()
}

fn sha256_of(path: &Path) -> Result<String, BackupError> {
    let mut file = File::open(path).map_err(io(format!("reading {}", path.display())))?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        let n = file
            .read(&mut buf)
            .map_err(io(format!("reading {}", path.display())))?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    let mut hex = String::with_capacity(64);
    for byte in hasher.finalize() {
        // `write!` sur un `String` ne peut pas echouer.
        let _ = std::fmt::Write::write_fmt(&mut hex, format_args!("{byte:02x}"));
    }
    Ok(hex)
}

fn sidecar(path: &Path) -> PathBuf {
    let mut name = path
        .file_name()
        .map(std::ffi::OsStr::to_os_string)
        .unwrap_or_default();
    name.push(".sha256");
    path.with_file_name(name)
}

/// Ecrit la somme de controle a cote du fichier (`<somme>  <nom>`, lisible par `sha256sum -c`).
fn write_checksum(path: &Path) -> Result<(), BackupError> {
    let sum = sha256_of(path)?;
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    fs::write(sidecar(path), format!("{sum}  {name}\n")).map_err(io("writing the checksum"))
}

/// Verifie la somme de controle d'un fichier ; l'absence de somme n'est pas une erreur (sauvegarde copiee a la main), une somme
/// differente en est une.
fn verify_checksum(path: &Path) -> Result<(), BackupError> {
    let Ok(text) = fs::read_to_string(sidecar(path)) else {
        return Ok(());
    };
    let expected = text.split_whitespace().next().unwrap_or_default();
    if expected == sha256_of(path)? {
        Ok(())
    } else {
        Err(BackupError::Corrupt(path.to_path_buf()))
    }
}

/// Retire une ligne que PostgreSQL 16 ne comprend pas : `pg_dump` 17 ecrit `SET transaction_timeout = 0;`, parametre inconnu d'un
/// serveur plus ancien (ADR 0054).
fn is_unsupported_line(line: &[u8]) -> bool {
    line.starts_with(b"SET transaction_timeout")
}

fn tail(bytes: &[u8]) -> String {
    let text = String::from_utf8_lossy(bytes);
    let text = text.trim();
    let start = text.len().saturating_sub(600);
    text.get(start..).unwrap_or(text).to_string()
}

impl BackupService {
    #[must_use]
    pub fn new(config: &Config) -> Self {
        Self {
            dir: config.backup_dir.as_ref().map(PathBuf::from),
            retention: Duration::from_secs(u64::from(config.backup_retention_days) * 86_400),
            reductstore_src: config.reductstore_backup_src.as_ref().map(PathBuf::from),
            pg_dump: config.pg_dump_bin.clone(),
            psql: config.psql_bin.clone(),
            db: PgTarget::parse(config.database_url.expose()).map_err(|e| e.to_string()),
        }
    }

    /// La sauvegarde planifiee tourne-t-elle (destination connue) ?
    #[must_use]
    pub fn is_configured(&self) -> bool {
        self.dir.is_some()
    }

    fn dir(&self) -> Result<&Path, BackupError> {
        self.dir.as_deref().ok_or(BackupError::NoDestination)
    }

    fn target(&self) -> Result<&PgTarget, BackupError> {
        self.db
            .as_ref()
            .map_err(|e| BackupError::BadDatabaseUrl(e.clone()))
    }

    /// `pg_dump` -> gzip -> fichier (ecrit sous un nom temporaire puis renomme).
    fn dump_postgres(&self, dir: &Path, stamp: &str) -> Result<Artifact, BackupError> {
        let target = self.target()?;
        let folder = dir.join(POSTGRES_DIR);
        fs::create_dir_all(&folder).map_err(io("creating the backup folder"))?;
        let path = folder.join(format!("dominet_{stamp}.sql.gz"));
        let tmp = folder.join(format!(".dominet_{stamp}.sql.gz.tmp"));
        let mut cmd = Command::new(&self.pg_dump);
        cmd.args(["--clean", "--if-exists", "--no-owner", "--no-privileges"]);
        target.apply(&mut cmd);
        let mut child = cmd
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(io(format!("starting {}", self.pg_dump)))?;
        let mut stdout = child.stdout.take().expect("stdout requested");
        let mut stderr = child.stderr.take().expect("stderr requested");
        // Les erreurs sont lues a part : un tube plein bloquerait le client.
        let errors = std::thread::spawn(move || {
            let mut buf = vec![];
            let _ = stderr.read_to_end(&mut buf);
            buf
        });
        let copied = (|| -> std::io::Result<()> {
            let mut gz = GzEncoder::new(File::create(&tmp)?, Compression::default());
            std::io::copy(&mut stdout, &mut gz)?;
            gz.finish()?.sync_all()
        })();
        let status = child.wait().map_err(io("waiting for pg_dump"))?;
        let stderr = errors.join().unwrap_or_default();
        if !status.success() {
            let _ = fs::remove_file(&tmp);
            return Err(BackupError::Client {
                program: self.pg_dump.clone(),
                status: status.to_string(),
                stderr: tail(&stderr),
            });
        }
        copied.map_err(io("writing the database backup"))?;
        fs::rename(&tmp, &path).map_err(io("finalising the database backup"))?;
        write_checksum(&path)?;
        let bytes = fs::metadata(&path).map_err(io("backup size"))?.len();
        Ok(Artifact { path, bytes })
    }

    /// Archive tar.gz du volume ReductStore ; `None` sans source (ou source absente : signale, pas bloquant, comme la reference).
    fn archive_reductstore(
        &self,
        dir: &Path,
        stamp: &str,
    ) -> Result<Option<Artifact>, BackupError> {
        let Some(src) = &self.reductstore_src else {
            return Ok(None);
        };
        if !src.is_dir() {
            tracing::warn!(src = %src.display(), "ReductStore source not found: backup skipped");
            return Ok(None);
        }
        let folder = dir.join(REDUCTSTORE_DIR);
        fs::create_dir_all(&folder).map_err(io("creating the backup folder"))?;
        let path = folder.join(format!("reductstore_{stamp}.tar.gz"));
        let tmp = folder.join(format!(".reductstore_{stamp}.tar.gz.tmp"));
        let written = (|| -> std::io::Result<()> {
            let gz = GzEncoder::new(File::create(&tmp)?, Compression::default());
            let mut tarball = tar::Builder::new(gz);
            // Les fichiers qui disparaissent pendant la lecture (ReductStore ecrit) n'arretent pas la sauvegarde.
            tarball.follow_symlinks(false);
            tarball.append_dir_all(".", src)?;
            tarball.into_inner()?.finish()?.sync_all()
        })();
        if let Err(e) = written {
            let _ = fs::remove_file(&tmp);
            return Err(BackupError::Io {
                what: "archive du volume ReductStore".into(),
                source: e,
            });
        }
        fs::rename(&tmp, &path).map_err(io("finalising the ReductStore archive"))?;
        write_checksum(&path)?;
        let bytes = fs::metadata(&path).map_err(io("archive size"))?.len();
        Ok(Some(Artifact { path, bytes }))
    }

    /// Supprime les sauvegardes plus vieilles que la retention, **sauf la plus recente de chaque dossier** (un arret prolonge de la
    /// sauvegarde ne doit jamais laisser sans rien).
    fn prune(&self, dir: &Path, now: SystemTime) -> usize {
        let mut removed = 0;
        for kind in [POSTGRES_DIR, REDUCTSTORE_DIR] {
            let Ok(entries) = fs::read_dir(dir.join(kind)) else {
                continue;
            };
            let mut files: Vec<(SystemTime, PathBuf)> = entries
                .flatten()
                .filter(|e| {
                    let n = e.file_name().to_string_lossy().into_owned();
                    !n.starts_with('.') && !n.ends_with(".sha256")
                })
                .filter_map(|e| Some((e.metadata().ok()?.modified().ok()?, e.path())))
                .collect();
            files.sort();
            files.pop(); // la plus recente reste
            for (modified, path) in files {
                if now
                    .duration_since(modified)
                    .is_ok_and(|age| age > self.retention)
                {
                    let _ = fs::remove_file(sidecar(&path));
                    if fs::remove_file(&path).is_ok() {
                        tracing::info!(file = %path.display(), "expired backup deleted");
                        removed += 1;
                    }
                }
            }
        }
        removed
    }

    /// Sauvegarde complete (bloquante : a lancer dans `spawn_blocking`).
    ///
    /// # Errors
    /// Destination absente, client PostgreSQL en echec, disque plein...
    pub fn run_blocking(&self, now: DateTime<Utc>) -> Result<BackupReport, BackupError> {
        let dir = self.dir()?;
        let stamp = stamp(now);
        let postgres = self.dump_postgres(dir, &stamp)?;
        let reductstore = self.archive_reductstore(dir, &stamp)?;
        let pruned = self.prune(dir, SystemTime::now());
        Ok(BackupReport {
            postgres,
            reductstore,
            pruned,
        })
    }

    /// Sauvegardes presentes (la plus recente en dernier) : `(nature, chemin)`.
    #[must_use]
    pub fn list(&self) -> Vec<(&'static str, PathBuf)> {
        let Ok(dir) = self.dir() else {
            return vec![];
        };
        let mut out = vec![];
        for kind in [POSTGRES_DIR, REDUCTSTORE_DIR] {
            let mut names: Vec<PathBuf> = fs::read_dir(dir.join(kind))
                .into_iter()
                .flatten()
                .flatten()
                .map(|e| e.path())
                .filter(|p| {
                    let n = p
                        .file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_default();
                    !n.starts_with('.') && !n.ends_with(".sha256")
                })
                .collect();
            names.sort();
            out.extend(names.into_iter().map(|p| (kind, p)));
        }
        out
    }

    /// La sauvegarde la plus recente d'une nature (`postgres` ou `reductstore`).
    #[must_use]
    pub fn latest(&self, kind: &str) -> Option<PathBuf> {
        self.list()
            .into_iter()
            .rev()
            .find(|(k, _)| *k == kind)
            .map(|(_, p)| p)
    }

    /// Restaure la base depuis un fichier `.sql.gz` : la somme de controle est verifiee, puis `psql` rejoue le SQL **en une seule
    /// transaction** (`ON_ERROR_STOP`) : un echec ne laisse pas une base a moitie restauree. Detruit les tables de l'application
    /// (le dump les recree : `--clean --if-exists`).
    ///
    /// # Errors
    /// Fichier abime, base injoignable, SQL refuse.
    pub fn restore_postgres(&self, file: &Path) -> Result<(), BackupError> {
        verify_checksum(file)?;
        let target = self.target()?;
        let mut cmd = Command::new(&self.psql);
        cmd.args(["-X", "-q", "-v", "ON_ERROR_STOP=1", "--single-transaction"]);
        target.apply(&mut cmd);
        let mut child = cmd
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(io(format!("starting {}", self.psql)))?;
        let mut stdin = child.stdin.take().expect("stdin requested");
        let mut stderr = child.stderr.take().expect("stderr requested");
        let errors = std::thread::spawn(move || {
            let mut buf = vec![];
            let _ = stderr.read_to_end(&mut buf);
            buf
        });
        let path = file.to_path_buf();
        let feeder = std::thread::spawn(move || -> std::io::Result<()> {
            let mut lines = BufReader::new(GzDecoder::new(File::open(path)?));
            let mut line = vec![];
            loop {
                line.clear();
                if lines.read_until(b'\n', &mut line)? == 0 {
                    return Ok(());
                }
                if !is_unsupported_line(&line) {
                    stdin.write_all(&line)?;
                }
            }
        });
        let fed = feeder
            .join()
            .unwrap_or_else(|_| Err(std::io::Error::other("read interrupted")));
        let status = child.wait().map_err(io("waiting for psql"))?;
        let stderr = errors.join().unwrap_or_default();
        if !status.success() {
            return Err(BackupError::Client {
                program: self.psql.clone(),
                status: status.to_string(),
                stderr: tail(&stderr),
            });
        }
        fed.map_err(io("reading the backup"))
    }

    /// Extrait une archive ReductStore dans `dest` (ReductStore doit etre arrete, `dest` etre son dossier de donnees). Refuse un
    /// dossier non vide sans `force`.
    ///
    /// # Errors
    /// Somme de controle incorrecte, dossier non vide, archive illisible.
    pub fn restore_reductstore(file: &Path, dest: &Path, force: bool) -> Result<u64, BackupError> {
        verify_checksum(file)?;
        if dest.is_dir()
            && !force
            && fs::read_dir(dest)
                .map_err(io("reading the target folder"))?
                .next()
                .is_some()
        {
            return Err(BackupError::Refused(format!(
                "{} is not empty: stop ReductStore and empty it, or use --force",
                dest.display()
            )));
        }
        fs::create_dir_all(dest).map_err(io("creating the target folder"))?;
        let mut archive = tar::Archive::new(GzDecoder::new(
            File::open(file).map_err(io("opening the archive"))?,
        ));
        let mut count = 0;
        for entry in archive.entries().map_err(io("reading the archive"))? {
            let mut entry = entry.map_err(io("reading the archive"))?;
            // `unpack_in` refuse les chemins qui sortent du dossier cible.
            if entry.unpack_in(dest).map_err(io("extraction"))? {
                count += 1;
            }
        }
        Ok(count)
    }
}

/// Prochaine heure de sauvegarde : `hour`:00 locale, aujourd'hui si elle n'est pas passee, demain sinon.
#[must_use]
pub fn next_run(now: DateTime<Utc>, tz: chrono_tz::Tz, hour: u32) -> DateTime<Utc> {
    use chrono::{Days, Timelike};
    let local = now.with_timezone(&tz);
    let mut day = local.date_naive();
    for _ in 0..4 {
        // Heure inexistante (saut d'heure) : la suivante.
        if let Some(at) = tz
            .from_local_datetime(&day.and_hms_opt(hour, 0, 0).unwrap_or_default())
            .earliest()
        {
            let at = at.with_timezone(&Utc);
            if at > now {
                return at;
            }
        }
        day = day.checked_add_days(Days::new(1)).unwrap_or(day);
    }
    now + chrono::Duration::hours(24 - i64::from(local.hour().min(23)))
}

/// Gauge « derniere sauvegarde reussie » reprise des fichiers a l'amorcage (le processus a pu redemarrer depuis).
fn init_metrics(service: &BackupService) {
    if let Some(newest) = service
        .latest(POSTGRES_DIR)
        .and_then(|p| p.metadata().ok())
        .and_then(|m| m.modified().ok())
        .and_then(|t| t.duration_since(SystemTime::UNIX_EPOCH).ok())
    {
        crate::metrics::backup_success_at(newest.as_secs_f64());
    }
}

/// Une sauvegarde, avec metriques et journal (bloquante dans `spawn_blocking`).
///
/// # Errors
/// Voir [`BackupService::run_blocking`].
pub async fn run_once(service: BackupService) -> Result<BackupReport, BackupError> {
    let started = std::time::Instant::now();
    let result = tokio::task::spawn_blocking(move || service.run_blocking(Utc::now()))
        .await
        .map_err(|e| BackupError::Refused(format!("task interrupted: {e}")))?;
    match &result {
        Ok(r) => {
            crate::metrics::backup_finished(
                true,
                started.elapsed(),
                r.postgres.bytes,
                r.reductstore.as_ref().map(|a| a.bytes),
            );
            tracing::info!(
                postgres = %r.postgres.path.display(),
                postgres_bytes = r.postgres.bytes,
                reductstore_bytes = r.reductstore.as_ref().map(|a| a.bytes),
                pruned = r.pruned,
                "backup finished"
            );
        }
        Err(e) => {
            crate::metrics::backup_finished(false, started.elapsed(), 0, None);
            tracing::error!(error = %e, "backup failed");
        }
    }
    result
}

/// Sauvegarde quotidienne a `BACKUP_HOUR` (heure locale), dans le planificateur ; sans `BACKUP_DIR`, rien ne tourne.
pub fn spawn(state: &AppState) {
    let service = BackupService::new(&state.config);
    if !service.is_configured() {
        return;
    }
    init_metrics(&service);
    let tz: chrono_tz::Tz = state
        .config
        .timezone
        .parse()
        .unwrap_or(chrono_tz::Europe::Paris);
    let hour = state.config.backup_hour;
    tokio::spawn(async move {
        tracing::info!(hour, %tz, "daily backup scheduled");
        loop {
            let at = next_run(Utc::now(), tz, hour);
            let wait = (at - Utc::now())
                .to_std()
                .unwrap_or(Duration::from_secs(60));
            tokio::time::sleep(wait).await;
            // Un echec est journalise et compte, la suivante aura lieu le lendemain.
            let _ = run_once(service.clone()).await;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_database_address_is_split_without_ever_putting_the_password_on_a_command_line() {
        let t = PgTarget::parse(
            "postgres://dominet:p%40ss%2Fword@db.local:5433/dominet?sslmode=require",
        )
        .unwrap();
        assert_eq!(
            (
                t.host.as_str(),
                t.port,
                t.user.as_str(),
                t.database.as_str()
            ),
            ("db.local", 5433, "dominet", "dominet")
        );
        assert_eq!(t.password, "p@ss/word");
        assert_eq!(t.sslmode.as_deref(), Some("require"));
        assert!(
            !format!("{t:?}").contains("p@ss"),
            "le mot de passe ne figure pas dans Debug"
        );
        assert!(PgTarget::parse("mysql://u@h/d").is_err());
        assert!(PgTarget::parse("postgres://u:p@h").is_err());
        assert_eq!(PgTarget::parse("postgres://u@h/d").unwrap().port, 5432);
    }

    #[test]
    fn only_the_setting_an_older_server_refuses_is_dropped() {
        assert!(is_unsupported_line(b"SET transaction_timeout = 0;\n"));
        assert!(!is_unsupported_line(b"SET statement_timeout = 0;\n"));
        assert!(!is_unsupported_line(
            b"INSERT INTO t VALUES ('SET transaction_timeout');\n"
        ));
    }

    #[test]
    fn the_next_run_is_today_if_the_hour_has_not_come_and_tomorrow_otherwise() {
        let paris = chrono_tz::Europe::Paris;
        let at = |y, m, d, h| Utc.with_ymd_and_hms(y, m, d, h, 0, 0).unwrap();
        // 2026-11-09 01:00 UTC = 02:00 a Paris (hiver, UTC+1) : 03:00 locale = 02:00 UTC le meme jour.
        assert_eq!(next_run(at(2026, 11, 9, 0), paris, 3), at(2026, 11, 9, 2));
        // 10:00 UTC : l'heure est passee, demain.
        assert_eq!(next_run(at(2026, 11, 9, 10), paris, 3), at(2026, 11, 10, 2));
        // Ete (UTC+2) : 03:00 locale = 01:00 UTC.
        assert_eq!(next_run(at(2026, 7, 1, 0), paris, 3), at(2026, 7, 1, 1));
        // Heure exactement atteinte : on attend le lendemain (pas de boucle de sauvegardes).
        assert_eq!(next_run(at(2026, 11, 9, 2), paris, 3), at(2026, 11, 10, 2));
        // Saut d'heure (31 mars 2026, 02:30 n'existe pas) : la sauvegarde de 02:00 locale a lieu quand meme un jour proche.
        let gap = next_run(at(2026, 3, 30, 12), paris, 2);
        assert!(gap > at(2026, 3, 30, 12) && gap < at(2026, 4, 2, 0));
    }
}
