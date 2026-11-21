# ADR 0054 — Sauvegardes et restauration


Statut : accepté (R7 lot 4, décisions D5, D6 et D12 du plan).

## Contexte
Une sauvegarde nocturne (3 h) de la base et du volume ReductStore, gardée 7 jours, ne vaut que si elle est **restaurable** : la base est PostgreSQL et ce qui compte est de pouvoir restaurer, pas seulement sauvegarder.

## Décisions
- **Base : `pg_dump`** (SQL, `--clean --if-exists --no-owner --no-privileges`), compressé en gzip, écrit sous un nom temporaire puis renommé (jamais de fichier à moitié écrit). Format standard : restaurable à la main (`gunzip -c f.sql.gz | psql`). Le client est dans l'image (`postgresql-client`).
- **Volume ReductStore : archive tar.gz** du dossier de données monté en **lecture seule** (`REDUCTSTORE_BACKUP_SRC`), comme la référence. Limite assumée : la cohérence est « au mieux » pendant que ReductStore écrit (il écrit des blocs immuables puis un index) ; la restauration a été **vérifiée avec de vrais enregistrements relus par un second ReductStore**.
- **Contrôle d'intégrité** : chaque fichier a sa somme SHA-256 à côté (`*.sha256`, lisible par `sha256sum -c`) ; une restauration la vérifie **avant** de toucher à quoi que ce soit.
- **Mot de passe de la base** : transmis aux clients PostgreSQL par l'environnement du processus fils (`PGPASSWORD`), **jamais** sur la ligne de commande (visible dans `ps`).
- **Restauration** : `dominet restore <fichier>|--latest postgres --yes` rejoue le SQL dans **une seule transaction** (`ON_ERROR_STOP`) : un échec ne laisse pas une base à moitié restaurée ; `--yes` est obligatoire (elle remplace les données). `restore --latest reductstore --to <dossier>` extrait l'archive (ReductStore arrêté ; refus d'un dossier non vide sans `--force` ; les chemins qui sortent du dossier cible sont ignorés).
- **Ligne écartée à la restauration** : `pg_dump` 17 (celui de Debian 13, dans l'image) écrit `SET transaction_timeout = 0;`, paramètre **inconnu de PostgreSQL 16** (le serveur de la pile) : la restauration échouerait. Constaté sur le vrai binaire ; la seule ligne `SET transaction_timeout…` est retirée du flux (testé).
- **Rétention** : `BACKUP_RETENTION_DAYS` (7), appliquée après chaque sauvegarde ; **la plus récente de chaque nature n'est jamais supprimée** (un arrêt prolongé du job ne doit pas laisser sans rien).
- **Planification** (D12) : dans le planificateur (rôle `scheduler`), tous les jours à `BACKUP_HOUR` (3, heure locale, saut d'heure géré), actif seulement si `BACKUP_DIR` est renseigné. `dominet backup` la déclenche à la main, `dominet backups` liste, `dominet restore` restaure.
- **Supervision** : métriques `dominet_backup_runs_total{result}`, `…_last_success_timestamp_seconds` (reprise des fichiers au démarrage), `…_last_duration_seconds`, `…_last_size_bytes{kind}` ; règles livrées `BackupTooOld` (plus de 36 h) et `BackupFailing`. Sans `BACKUP_DIR`, la métrique n'existe pas et rien n'alerte.

## Hors sauvegarde — à savoir
- Les secrets chiffrés (mots de passe de caméras, clés SMS) ne sont lisibles qu'avec la **`DOMINET_MASTER_KEY` d'origine** : elle se garde **à part** (une sauvegarde de la base sans elle perd ces secrets).
- Les segments vidéo encore sur disque (avant archivage), les snapshots, les images annotées, les tableaux Grafana générés et les volumes Prometheus / Grafana ne sont pas sauvegardés : ce sont des données recalculables ou de courte durée ; le compose (lot 5) dit lesquels sont des volumes.

## Contrôles
- Tests d'intégration (`tests/backup.rs`, `pg_dump` du conteneur de dev) : sauvegarde → restauration dans une **autre** base → empreinte de **chaque table** et schéma identiques ; accents et tirets conservés ; restauration par-dessus une base existante ; fichier abîmé refusé sans toucher à la base ; client en échec sans fichier résiduel ; rétention ; archive du volume (aller-retour, chemin `../` refusé).
- `tools/check-backup.sh` : avec l'**image** (client 17) contre PostgreSQL 16 et de **vrais ReductStore** jetables : mêmes contrôles, plus relecture octet par octet des enregistrements par un second ReductStore.

## Conséquences
- La procédure de restauration est dans `docs/backups.md`. Essai sur l'hôte réel : R8.
