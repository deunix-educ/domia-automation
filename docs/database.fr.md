# Base de données

PostgreSQL 16, accessible par `sqlx`. Les migrations sont dans `crates/dominet-store/migrations/`, embarquées dans le
binaire et appliquées par `dominet migrate` (jamais automatiquement au démarrage). Relancer la commande est sans effet.

## Conventions

- Clés primaires `BIGINT GENERATED ALWAYS AS IDENTITY` (le contrat WebSocket utilise des `device_id` entiers).
- Horodatages `timestamptz` en UTC ; l'affichage se fait en `Europe/Paris` (réglage `TZ`).
- Pas d'énumérations PostgreSQL : `TEXT` + contrainte `CHECK` (plus simple à faire évoluer).
- `updated_at` tenu à jour par un déclencheur (`set_updated_at`) sur les tables modifiables.
- Les secrets ne sont jamais en clair : mots de passe en argon2id, valeurs sensibles des réglages chiffrées (ADR 0011).

## Tables (R1 à R3)

| Table | Rôle  |
|---|--- |
| `users` | comptes (`role` = `admin` ou `user`, mobile pour les SMS) ; unicité du nom insensible à la casse  |
| `session` | sessions côté serveur (`user_id` permet de les révoquer toutes)  |
| `login_attempt` | tentatives de connexion pour le blocage progressif  |
| `app_setting` | réglages typés (`str`/`int`/`float`/`bool`), `is_secret`  |
| `navbar_item` | navigation (`admin_only`, `sort_order`, `url` = chemin ou URL externe)  |
| `location` | emplacements (pièces, zones)  |
| `message`, `message_recipient` | messagerie interne (canal, statuts d'envoi par destinataire)  |
| `android_device`, `sms_message`, `call_log`, `sms_outbox` | relais Android (jeton UUID régénérable, **stocké haché** SHA-256 ; file d'envoi avec accusés, ADR 0037)  |
| `schedule_job` | planificateur interne : intervalle, cron ou solaire  |
| `dashboard` | tableaux de bord d'un compte (un par défaut au plus, nom ≤ 64)  |
| `widget` | widgets d'un tableau : type, titre, références (`device_id`, `scene_id`, `camera_uuid`, **sans clé étrangère** jusqu'à R3/R5/R6), présentation, `config` JSON, `content` HTML, rectangle `grid_col`/`grid_row`/`width`/`height` sur 12 colonnes  |

| `device_group`, `device_label` | groupes d'appareils (emplacement, icône) ; libellés des clés d'état (nom, icône Lucide, unité) | `automation.DeviceGroup`, `DeviceLabel` |
| `device` | appareil : nom Zigbee2MQTT unique, type, prefixe de topic, **état** (JSONB remplacé à chaque message), `last_seen`, disponibilité, `exposes`, clés suivies / masquées (`TEXT[]`) | `automation.Device` |
| `device_state_log` | historique des clés suivies, une ligne par appareil, clé et seconde | `automation.DeviceStateLog` |
| `device_image` | image téléversée d'un appareil (JPEG réduit, en base) | `image_file` |
| `mqtt_source`, `weather_station` | abonnements MQTT supplémentaires (Linky, SolarMax…) ; stations Open-Meteo | `automation.MqttSource`, `weather.WeatherStation` |

### Scènes, automations, alertes, énergie (R4, migration 0009, ADR 0031)

| Table | Rôle  |
|---|--- |
| `scene`, `scene_action` | scènes (nom **unique**, icône, emplacement) et leurs commandes ordonnées (`position`) vers des appareils  |
| `automation` | déclencheur / condition / action en **JSON** (`trigger_type` + `trigger_config`…), script Rhai, `last_triggered`, `trigger_count`  |
| `automation_run` | journal d'exécution (instant, cause, statut, durée, message)  |
| `alert_rule`, `alert` | règles (opérateur, seuil, sévérité, cooldown, canaux) et alertes (`firing` / `resolved` / `acknowledged`)  |
| `energy_daily_log` | bilan journalier Linky HP / HC, solaire, PAC (Wh)  |

`widget.scene_id` devient une clé étrangère vers `scene` (`ON DELETE SET NULL`) ; `schedule_job` gagne `automation_id` et admet les neuf événements solaires.

### Relais Android (R5, migration 0010, ADR 0037)

- `android_device.token` disparaît au profit de `token_hash` (SHA-256 hexadécimal, unique) : le jeton est produit par `gen_random_uuid()` en SQL et n'est montré **qu'une fois** (création ou régénération). Les appareils existants gardent leur jeton (empreinte calculée par la migration).
- `sms_outbox` : un SMS demandé à un appareil (identifiant propre = `message_id` du protocole, `pending` / `sent` / `failed`, erreur, accusé) ; origine = un SMS du navigateur **ou** un destinataire de la messagerie, jamais les deux.

## Appareils (R3, migration 0008, ADR 0025)

- `device.state` est **remplacé** à chaque message (pas de fusion) ; les opérations de l'écoute MQTT sont des méthodes du dépôt, une instruction SQL chacune, idempotentes.
- `device_state_log` : `UNIQUE(device_id, key, ts)` à la seconde, écriture par lot avec remplacement ; purge par date (rétention configurable).
- `widget.device_id` → `device(id) ON DELETE SET NULL` ; une référence à un appareil inexistant est refusée.

## Tableaux de bord et widgets (R2, migration 0007)

- Extension `btree_gist` (créée par la migration) : permet la contrainte d'exclusion `widget_no_overlap`, qui interdit à deux widgets d'un même tableau de se chevaucher
  (rectangles en cellules, bornes incluses ; des widgets contigus passent). L'invariant est donc garanti par la base, même si deux déplacements arrivent en même temps.
- Un seul tableau par défaut par compte (index unique partiel) ; suppression en cascade utilisateur → tableaux → widgets.
- Les écritures de widgets d'un tableau sont sérialisées par un verrou de ligne sur le tableau (sinon PostgreSQL signale un `deadlock` entre deux insertions concurrentes).
- Détails et raisons : [ADR 0018](adr/0018-dashboard-schema.md).

## Accès depuis le code

- Le domaine (`dominet-domain`) définit les types validés (`Username`, `SettingKey`, `NavbarDraft`...) et les ports
  `UserRepo`, `SettingRepo`, `NavbarRepo`, `LocationRepo`, `DashboardRepo`, `WidgetRepo` ; `dominet-store` les implémente avec `PgStore`
  (`store.users()`, `store.settings()`, `store.navbar()`, `store.locations()`, `store.dashboards()`, `store.widgets()`, `store.devices()`, `store.device_groups()`,
  `store.device_labels()`, `store.state_log()`, `store.device_images()`, `store.mqtt_sources()`, `store.weather()`, `store.scenes()`, `store.automations()`, `store.automation_runs()`,
  `store.alert_rules()`, `store.alerts()`, `store.energy()`).
- Les requêtes sont vérifiées à la compilation (`sqlx::query!`). Les métadonnées hors ligne sont dans `.sqlx/` (suivi par git) :
  après toute modification d'une requête ou d'une migration, lancer `make sqlx-prepare` ; `make sqlx-check` (inclus dans
  `make ci`) vérifie qu'elles sont à jour.
