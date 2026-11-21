// SPDX-License-Identifier: AGPL-3.0-or-later
//! Sauvegarde et restauration (R7 lot 4, ADR 0054) : la base est sauvegardee par `pg_dump` (celui du conteneur de la stack de dev),
//! restauree dans une AUTRE base jetable, et les deux sont comparees table par table ; sommes de controle, retention, archive du volume
//! ReductStore (aller-retour, chemins hors dossier refuses) et echecs de clients.

use std::fs;
use std::path::{Path, PathBuf};

use chrono::{TimeZone, Utc};
use dominet_server::backup::{BackupError, BackupService};
use dominet_server::config::Config;
use sqlx::{Executor, PgPool, Row};

fn bin(name: &str) -> String {
    format!("{}/tests/bin/{name}", env!("CARGO_MANIFEST_DIR"))
}

fn tmp(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("dominet-backup-{tag}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

/// Adresse de la base de test (ou d'une autre base du meme serveur) : celle de `DATABASE_URL` (stack de dev ou base jetable du hook
/// pre-push), avec son identifiant et son mot de passe.
fn url_of(pool: &PgPool, database: Option<&str>) -> String {
    let mut url = url::Url::parse(&std::env::var("DATABASE_URL").expect("DATABASE_URL")).unwrap();
    let name = database.map_or_else(
        || pool.connect_options().get_database().unwrap().to_string(),
        str::to_string,
    );
    url.set_path(&name);
    url.to_string()
}

fn config(database_url: &str, dir: &Path, src: Option<&Path>, pg_dump: &str, psql: &str) -> Config {
    let env = [
        ("DATABASE_URL", database_url.to_string()),
        ("BACKUP_DIR", dir.display().to_string()),
        ("PG_DUMP_BIN", pg_dump.to_string()),
        ("PSQL_BIN", psql.to_string()),
        ("BACKUP_RETENTION_DAYS", "7".to_string()),
    ];
    let src = src.map(|p| p.display().to_string());
    Config::from_lookup(|k| {
        if k == "REDUCTSTORE_BACKUP_SRC" {
            return src.clone();
        }
        env.iter()
            .find(|(key, _)| *key == k)
            .map(|(_, v)| v.clone())
    })
    .unwrap()
}

async fn fill(pool: &PgPool) {
    pool.execute(
        "INSERT INTO location (name, description) VALUES ('Salon', 'Pièce « principale » — étage 1'), ('Cave', E'ligne 1\\nligne 2 ''quote''')",
    )
    .await
    .unwrap();
    pool.execute(
        "INSERT INTO device (friendly_name, device_type, state, tracked_keys) VALUES
           ('Sonde', 'sensor', '{\"temperature\": 21.5, \"msg\": \"é — ü\"}', '{temperature,humidity}'),
           ('Lampe', 'light', '{}', '{}')",
    )
    .await
    .unwrap();
    pool.execute("INSERT INTO device_state_log (device_id, key, ts, value_num) SELECT id, 'temperature', now() - interval '1 minute', 21.5 FROM device WHERE friendly_name = 'Sonde'")
        .await
        .unwrap();
}

/// Empreinte du contenu de chaque table (lignes triees) : deux bases identiques ont les memes.
async fn fingerprint(pool: &PgPool) -> Vec<(String, i64, String)> {
    let tables: Vec<String> = sqlx::query_scalar(
        "SELECT table_name::text FROM information_schema.tables WHERE table_schema = 'public' AND table_type = 'BASE TABLE' ORDER BY 1",
    )
    .fetch_all(pool)
    .await
    .unwrap();
    let mut out = vec![];
    for t in tables {
        let row = sqlx::query(sqlx::AssertSqlSafe(format!(
            "SELECT count(*)::bigint, md5(COALESCE(string_agg(x::text, E'\\n' ORDER BY x::text), '')) FROM {t} x"
        )))
        .fetch_one(pool)
        .await
        .unwrap();
        out.push((t, row.get::<i64, _>(0), row.get::<String, _>(1)));
    }
    out
}

/// Colonnes et index : la restauration recree le meme schema.
async fn schema(pool: &PgPool) -> Vec<String> {
    let mut out: Vec<String> = sqlx::query_scalar(
        "SELECT table_name || '.' || column_name || ':' || data_type || ':' || is_nullable FROM information_schema.columns WHERE table_schema = 'public'",
    )
    .fetch_all(pool)
    .await
    .unwrap();
    out.extend(
        sqlx::query_scalar::<_, String>(
            "SELECT indexname FROM pg_indexes WHERE schemaname = 'public'",
        )
        .fetch_all(pool)
        .await
        .unwrap(),
    );
    out.sort();
    out
}

async fn scratch_database(admin: &PgPool, name: &str) -> PgPool {
    admin
        .execute(sqlx::AssertSqlSafe(format!(
            "DROP DATABASE IF EXISTS {name} WITH (FORCE)"
        )))
        .await
        .unwrap();
    admin
        .execute(sqlx::AssertSqlSafe(format!("CREATE DATABASE {name}")))
        .await
        .unwrap();
    let opts = admin.connect_options().as_ref().clone().database(name);
    PgPool::connect_with(opts).await.unwrap()
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn a_backup_restores_into_another_database_with_the_same_schema_and_data(pool: PgPool) {
    fill(&pool).await;
    let dir = tmp("roundtrip");
    let svc = BackupService::new(&config(
        &url_of(&pool, None),
        &dir,
        None,
        &bin("pg_dump"),
        &bin("psql"),
    ));
    let report = svc
        .run_blocking(Utc.with_ymd_and_hms(2026, 11, 9, 3, 0, 0).unwrap())
        .unwrap();
    assert!(
        report
            .postgres
            .path
            .ends_with("postgres/dominet_20261109_030000.sql.gz")
    );
    assert!(report.postgres.bytes > 1000 && report.reductstore.is_none());
    let sum =
        fs::read_to_string(dir.join("postgres/dominet_20261109_030000.sql.gz.sha256")).unwrap();
    assert!(
        sum.trim_end().ends_with("  dominet_20261109_030000.sql.gz")
            && sum.split_whitespace().next().unwrap().len() == 64
    );
    assert!(
        fs::read_dir(dir.join("postgres"))
            .unwrap()
            .flatten()
            .all(|e| !e.file_name().to_string_lossy().ends_with(".tmp"))
    );

    // Restauration dans une base VIERGE (la sauvegarde recree tout), puis comparaison.
    let admin = PgPool::connect_with(pool.connect_options().as_ref().clone().database("postgres"))
        .await
        .unwrap();
    let name = format!("bk_restore_{}", std::process::id());
    let target = scratch_database(&admin, &name).await;
    let restore = BackupService::new(&config(
        &url_of(&pool, Some(&name)),
        &dir,
        None,
        &bin("pg_dump"),
        &bin("psql"),
    ));
    restore.restore_postgres(&report.postgres.path).unwrap();
    assert_eq!(
        fingerprint(&target).await,
        fingerprint(&pool).await,
        "memes tables, memes lignes"
    );
    assert_eq!(
        schema(&target).await,
        schema(&pool).await,
        "meme schema et memes index"
    );
    let note: String = sqlx::query_scalar("SELECT description FROM location WHERE name = 'Salon'")
        .fetch_one(&target)
        .await
        .unwrap();
    assert_eq!(
        note, "Pièce « principale » — étage 1",
        "les accents et les tirets survivent"
    );

    // Une deuxieme restauration par-dessus (--clean) donne le meme resultat : on peut restaurer sur une base en service.
    restore.restore_postgres(&report.postgres.path).unwrap();
    assert_eq!(fingerprint(&target).await, fingerprint(&pool).await);
    // `latest` et la liste voient la sauvegarde.
    assert_eq!(svc.latest("postgres").unwrap(), report.postgres.path);
    assert_eq!(
        svc.list().len(),
        1,
        "les sommes de controle ne sont pas des sauvegardes"
    );

    target.close().await;
    admin
        .execute(sqlx::AssertSqlSafe(format!(
            "DROP DATABASE IF EXISTS {name} WITH (FORCE)"
        )))
        .await
        .unwrap();
    let _ = fs::remove_dir_all(&dir);
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn a_damaged_backup_is_refused_before_anything_is_touched(pool: PgPool) {
    fill(&pool).await;
    let dir = tmp("corrupt");
    let svc = BackupService::new(&config(
        &url_of(&pool, None),
        &dir,
        None,
        &bin("pg_dump"),
        &bin("psql"),
    ));
    let report = svc.run_blocking(Utc::now()).unwrap();
    let mut bytes = fs::read(&report.postgres.path).unwrap();
    let middle = bytes.len() / 2;
    bytes[middle] ^= 0xff;
    fs::write(&report.postgres.path, bytes).unwrap();
    pool.execute("DELETE FROM location").await.unwrap();
    let err = svc.restore_postgres(&report.postgres.path).unwrap_err();
    assert!(matches!(err, BackupError::Corrupt(_)), "{err}");
    let left: i64 = sqlx::query_scalar("SELECT count(*) FROM location")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(left, 0, "la base n'a pas ete touchee");
    let _ = fs::remove_dir_all(&dir);
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn a_failing_client_leaves_no_half_written_backup(pool: PgPool) {
    let dir = tmp("failing");
    let svc = BackupService::new(&config(
        &url_of(&pool, None),
        &dir,
        None,
        "/bin/false",
        &bin("psql"),
    ));
    let err = svc.run_blocking(Utc::now()).unwrap_err();
    assert!(matches!(err, BackupError::Client { .. }), "{err}");
    assert_eq!(svc.list().len(), 0);
    assert!(
        fs::read_dir(dir.join("postgres")).map_or(true, |mut d| d.next().is_none()),
        "aucun fichier, pas meme temporaire"
    );
    // Programme introuvable : erreur claire, pas de panique.
    let svc = BackupService::new(&config(
        &url_of(&pool, None),
        &dir,
        None,
        "/nonexistent/pg_dump",
        &bin("psql"),
    ));
    assert!(matches!(
        svc.run_blocking(Utc::now()),
        Err(BackupError::Io { .. })
    ));
    // Pas de destination : refus net.
    let none =
        Config::from_lookup(|k| (k == "DATABASE_URL").then(|| "postgres://u:p@h/d".to_string()))
            .unwrap();
    assert!(matches!(
        BackupService::new(&none).run_blocking(Utc::now()),
        Err(BackupError::NoDestination)
    ));
    let _ = fs::remove_dir_all(&dir);
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn old_backups_are_pruned_but_the_newest_of_each_kind_always_stays(pool: PgPool) {
    let dir = tmp("prune");
    let src = tmp("prune-src");
    fs::write(src.join("a"), "x").unwrap();
    for kind in ["postgres", "reductstore"] {
        fs::create_dir_all(dir.join(kind)).unwrap();
    }
    let old = |path: PathBuf, days: u64| {
        fs::write(&path, "ancien").unwrap();
        fs::write(
            path.with_file_name(format!(
                "{}.sha256",
                path.file_name().unwrap().to_string_lossy()
            )),
            "somme",
        )
        .unwrap();
        let t = std::time::SystemTime::now() - std::time::Duration::from_secs(days * 86_400);
        fs::File::options()
            .write(true)
            .open(&path)
            .unwrap()
            .set_modified(t)
            .unwrap();
    };
    old(dir.join("postgres/dominet_20260101_030000.sql.gz"), 30);
    old(dir.join("postgres/dominet_20260102_030000.sql.gz"), 20);
    old(dir.join("postgres/dominet_20261105_030000.sql.gz"), 3);
    old(
        dir.join("reductstore/reductstore_20260101_030000.tar.gz"),
        40,
    );
    let svc = BackupService::new(&config(
        &url_of(&pool, None),
        &dir,
        Some(&src),
        &bin("pg_dump"),
        &bin("psql"),
    ));
    let report = svc.run_blocking(Utc::now()).unwrap();
    assert_eq!(
        report.pruned, 3,
        "les deux vieux dumps et l'ancienne archive (remplacee par la nouvelle)"
    );
    let names: Vec<String> = svc
        .list()
        .into_iter()
        .map(|(_, p)| p.file_name().unwrap().to_string_lossy().into_owned())
        .collect();
    assert!(
        names.contains(&"dominet_20261105_030000.sql.gz".to_string()),
        "la sauvegarde de 3 jours reste : {names:?}"
    );
    assert!(
        !names
            .iter()
            .any(|n| n.contains("20260101") || n.contains("20260102")),
        "{names:?}"
    );
    assert!(
        !dir.join("postgres/dominet_20260101_030000.sql.gz.sha256")
            .exists(),
        "la somme part avec son fichier"
    );

    // Meme si TOUT est plus vieux que la retention, la plus recente reste : on ne se retrouve jamais sans rien.
    for (_, p) in svc.list() {
        let t = std::time::SystemTime::now() - std::time::Duration::from_hours(9600);
        fs::File::options()
            .write(true)
            .open(p)
            .unwrap()
            .set_modified(t)
            .unwrap();
    }
    let failing = BackupService::new(&config(
        &url_of(&pool, None),
        &dir,
        None,
        "/bin/false",
        &bin("psql"),
    ));
    let _ = failing.run_blocking(Utc::now());
    assert_ne!(svc.list().len(), 0);
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&src);
}

#[test]
fn the_reductstore_volume_survives_a_round_trip_and_never_writes_outside_its_target() {
    let svc_cfg =
        Config::from_lookup(|k| (k == "DATABASE_URL").then(|| "postgres://u:p@h/d".to_string()))
            .unwrap();
    let _ = svc_cfg;
    let src = tmp("rs-src");
    fs::create_dir_all(src.join("bucket/entry")).unwrap();
    fs::create_dir_all(src.join("vide")).unwrap();
    fs::write(src.join("bucket/entry/0.blk"), vec![7u8; 100_000]).unwrap();
    fs::write(src.join("bucket/settings.json"), r#"{"quota": 1}"#).unwrap();
    let dir = tmp("rs-backup");
    let cfg = config(
        "postgres://u:p@127.0.0.1:1/d",
        &dir,
        Some(&src),
        "/bin/true",
        "/bin/true",
    );
    let svc = BackupService::new(&cfg);
    // `/bin/true` tient lieu de `pg_dump` : sortie vide, valide (sauvegarde vide) ; seule l'archive du volume nous interesse ici.
    let report = svc.run_blocking(Utc::now()).unwrap();
    let archive = report.reductstore.expect("archive du volume").path;

    let dest = tmp("rs-dest");
    fs::remove_dir_all(&dest).unwrap();
    let n = BackupService::restore_reductstore(&archive, &dest, false).unwrap();
    assert!(n >= 4, "dossiers et fichiers extraits ({n})");
    assert_eq!(
        fs::read(dest.join("bucket/entry/0.blk")).unwrap(),
        vec![7u8; 100_000]
    );
    assert_eq!(
        fs::read_to_string(dest.join("bucket/settings.json")).unwrap(),
        r#"{"quota": 1}"#
    );
    assert!(
        dest.join("vide").is_dir(),
        "un dossier vide est restaure aussi"
    );
    // Cible non vide : refus, sauf --force.
    assert!(matches!(
        BackupService::restore_reductstore(&archive, &dest, false),
        Err(BackupError::Refused(_))
    ));
    assert!(BackupService::restore_reductstore(&archive, &dest, true).is_ok());

    // Une archive piegee (chemin `../`) n'ecrit rien hors du dossier cible.
    let evil = tmp("rs-evil");
    let evil_tar = evil.join("evil.tar.gz");
    {
        let gz = flate2::write::GzEncoder::new(
            fs::File::create(&evil_tar).unwrap(),
            flate2::Compression::default(),
        );
        let mut b = tar::Builder::new(gz);
        let mut h = tar::Header::new_gnu();
        let name = b"../evil.txt";
        h.as_old_mut().name[..name.len()].copy_from_slice(name);
        h.set_size(4);
        h.set_cksum();
        b.append(&h, &b"evil"[..]).unwrap();
        b.into_inner().unwrap().finish().unwrap();
    }
    let target = evil.join("inside");
    let _ = BackupService::restore_reductstore(&evil_tar, &target, false);
    assert!(
        !evil.join("evil.txt").exists(),
        "rien n'a ete ecrit hors de la cible"
    );
    for d in [src, dir, dest, evil] {
        let _ = fs::remove_dir_all(d);
    }
}
