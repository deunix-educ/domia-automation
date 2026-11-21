# 0043 — Schéma de la surveillance : caméras, enregistrements, événements


- Statut : accepté
- Date : 2026-11-07

## Contexte
R6 porte la surveillance de la référence ( `Camera`, `PTZPreset`, `Recording`, `Snapshot`, `MotionEvent`, `DetectionEvent`, `DetectorConfig`). Plan R6 : D2 (secrets), D3 (identifiant), D9 (droits), D10 (enregistrement).

## Décision
- **Migration 0011** : les sept tables de la référence, avec des contraintes `CHECK` qui reprennent les règles du domaine (ports, fps, priorité, énumérations, confiance dans [0, 1]) : la base refuse ce que le domaine refuse. Suppression d'une caméra : tout ce qui lui appartient part en cascade ; suppression d'un emplacement : la caméra est détachée (`SET NULL`), comme la référence.
- **Identifiant public (D3)** : `camera.uuid`, un `UUID` tiré par la base (`gen_random_uuid()`), affiché en **32 caractères hexadécimaux** (`CameraUuid`). C'est le nom du path MediaMTX et la clé du widget (`camera_uuid`, sans clé étrangère depuis R3). La clé numérique reste la clé étrangère interne. *Écart avec la référence* : ses identifiants sont des slugs de 12 caractères ; l'import de R8 devra les remplacer (et mettre à jour les widgets) — à traiter à ce moment-là.
- **Secrets (D2)** : mot de passe RTSP, mot de passe ONVIF, mot de passe web et clé d'API web sont stockés **chiffrés** (`*_enc`, format `v1:` de l'ADR 0011). Le domaine ne voit qu'un `Sealed` (texte déjà chiffré, `Debug` masqué) : le chiffrement et le déchiffrement sont faits par le serveur (`SecretBox`, donnée associée propre à la caméra et au champ), jamais par le dépôt. Les URL qui contiennent un mot de passe n'existent qu'en mémoire (`source_url`) ; `masked_source_url` est la seule qui s'affiche ou se journalise.
- **Dépôts** : un port par agrégat (`CameraRepo`, `PtzPresetRepo`, `RecordingRepo`, `SnapshotRepo`, `EventRepo` pour mouvements et détections, `DetectorConfigRepo`). `Recording` suit `pending → uploading → done | error` ; `list_unarchived` ressort ce qui reste à pousser (reprise après un arrêt). `detector_config` est un singleton (`id = 1`) créé par la migration.
- **Champs libres** : le chemin du flux refuse espaces et `@`, l'hôte n'admet que lettres, chiffres, `.`, `-`, `:` — ils entrent dans une URL passée à MediaMTX et à `ffmpeg`.
- **Différences voulues avec la référence** : durée des segments en millisecondes (`duration_ms`) au lieu d'un `DurationField` ; classes YOLO en `TEXT[]` ; étiquettes de détection en `JSONB` validé comme tableau ; positions de preset `position` (la référence : `order`).

## Conséquences
- Une caméra importée depuis une autre installation aura des secrets à chiffrer et un nouvel identifiant.
- Changer `DOMINET_MASTER_KEY` rend les secrets des caméras illisibles, comme ceux des réglages (ADR 0011) : la caméra reste, les mots de passe sont à ressaisir.
