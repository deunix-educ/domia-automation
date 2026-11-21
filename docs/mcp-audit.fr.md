# R11 — M0 : audit des fonctions serveur et cadrage (2026-11-15)

Lot M0 de la feuille de route MCP (non conservée) : savoir **où vit la logique** de l'interface, ce que l'assistant peut en réutiliser, et trancher la technique (SDK, montage, transport, jetons). Décisions : [ADR 0058](adr/0058-mcp-server.md) (serveur) et [ADR 0059](adr/0059-mcp-access-tokens.md) (jetons). Le tableau complet est dans `tests/mcp/coverage.tsv` ; **un test de `make ci` échoue si une fonction serveur n'y est pas classée**.

## Ce que l'audit a mesuré
- **107 fonctions serveur** dans 20 fichiers de `dominet-web` (3 482 lignes de corps au total) : 75 réservées aux administrateurs, 25 à tout compte connecté, 7 liées à la session ou au compte courant (connexion, déconnexion, mot de passe, tableau de bord personnel).
- **Le schéma est uniforme** : `expect_context::<AppState>()`, un contrôle d'accès (`require_admin` / `require_user`), puis une logique qui appelle directement `AppState` et les dépôts. Seules 7 fonctions appellent directement `leptos_axum` : celles de la session et du compte courant. Aucun couplage profond : **la logique est extractible**.
- **Le pilotage est déjà réutilisable** : `send_device_command` et `set_permit_join` appellent `dominet_server::control`. **La configuration ne l'est pas encore** : les `save_*` (jusqu'à 98 lignes) contiennent la validation, l'écriture et le signal de rechargement. Il faudra les déplacer dans `dominet-server`, sous la forme `service(&AppState, &Acteur, entrée)`, pour que l'interface et MCP partagent le même code (lots M2 à M5).
- **Technique** : `rmcp` 3.10.0 (SDK officiel, Apache-2.0, Rust 1.88 minimum) est accepté par `cargo deny` et n'ajoute aucun avertissement à `cargo audit`. Le serveur est **sans session** (réponses JSON simples), protégé contre le DNS rebinding (en-tête `Host` limité à `localhost` et `ALLOWED_HOSTS`) et fonctionne avec un client de la révision précédente du protocole (testé).

## Couverture : 80 couvertes, 27 exclues
| Fonction serveur | Fichier | Garde | Outil MCP | Portée | Statut | Note |
|---|---|---|---|---|---|---|
| `list_alert_rules` | alert_rules_admin.rs | admin | `config_list(alert_rule)` | lecture | couvert |  |
| `save_alert_rule` | alert_rules_admin.rs | admin | `config_upsert(alert_rule)` | configuration | couvert |  |
| `delete_alert_rule` | alert_rules_admin.rs | admin | `config_delete(alert_rule)` | configuration | couvert | `confirm` et `dry_run` |
| `alerts_page` | alerts.rs | compte | `alerts_list` | lecture | couvert |  |
| `acknowledge_alert` | alerts.rs | compte | `alert_ack` | pilotage | couvert |  |
| `acknowledge_all_alerts` | alerts.rs | compte | `alert_ack(all)` | pilotage | couvert | tout acquitter : une carte, un clic côté assistant |
| `android_overview` | android.rs | compte | — | — | exclu | Android : canal de messagerie et jetons |
| `android_devices` | android.rs | compte | — | — | exclu | liste des téléphones et de leurs jetons |
| `add_android_device` | android.rs | compte | — | — | exclu | crée un jeton Android (secret) |
| `delete_android_device` | android.rs | compte | — | — | exclu | jetons Android |
| `regenerate_android_token` | android.rs | compte | — | — | exclu | jetons Android (secret) |
| `android_sms_page` | android.rs | compte | — | — | exclu | messagerie SMS exclue (décision du 2026-11-13) |
| `android_calls_page` | android.rs | compte | — | — | exclu | journal d'appels : données personnelles, messagerie exclue |
| `send_android_sms` | android.rs | compte | — | — | exclu | messagerie SMS exclue (décision du 2026-11-13) |
| `list_automations` | automations_admin.rs | admin | `automations_list` | lecture | couvert |  |
| `save_automation` | automations_admin.rs | admin | `config_upsert(automation)` | configuration | couvert | avec script Rhai : portée `administration` (voir constats) |
| `delete_automation` | automations_admin.rs | admin | `config_delete(automation)` | configuration | couvert | `confirm` et `dry_run` |
| `automation_runs` | automations_admin.rs | admin | `automation_runs` | lecture | couvert |  |
| `list_cameras` | camera_fns.rs | admin | `cameras_list` | lecture | couvert | sans identifiants |
| `list_camera_cards` | camera_fns.rs | compte | `cameras_list` | lecture | couvert |  |
| `save_camera` | camera_fns.rs | admin | `config_upsert(camera)` | administration | couvert | identifiants en écriture seule (M5) |
| `delete_camera` | camera_fns.rs | admin | `config_delete(camera)` | administration | couvert | `confirm` et `dry_run` |
| `sync_cameras` | camera_fns.rs | admin | `cameras_sync` | administration | couvert | synchronisation MediaMTX (décidé le 2026-11-16) |
| `sync_camera_onvif` | camera_fns.rs | admin | `cameras_sync` | administration | couvert | profils ONVIF (décidé le 2026-11-16) |
| `add_camera_by_ip` | camera_fns.rs | admin | — | — | exclu | saisit des identifiants à partir d'une adresse : interface seule (décidé le 2026-11-16) |
| `start_discovery` | camera_fns.rs | admin | — | — | exclu | balayage du réseau : déclenchement humain seulement |
| `camera_live` | camera_fns.rs | compte | — | — | exclu | flux vidéo : sans objet pour un assistant |
| `camera_night` | camera_fns.rs | admin | `camera_night` | pilotage | couvert | mode nuit (décidé le 2026-11-16) |
| `capture_snapshot` | camera_fns.rs | admin | `camera_snapshot` | pilotage | couvert | renvoie une image |
| `list_snapshots` | camera_fns.rs | compte | `camera_snapshot(list)` | lecture | couvert | liste des captures (décidé le 2026-11-16) |
| `delete_snapshot` | camera_fns.rs | admin | — | — | exclu | suppression : interface seule (décidé le 2026-11-16) |
| `list_recordings` | camera_fns.rs | compte | `recordings_list` | lecture | couvert |  |
| `delete_recording` | camera_fns.rs | admin | — | — | exclu | suppression : interface seule (décidé le 2026-11-16) |
| `set_manual_record` | camera_fns.rs | admin | `camera_record` | pilotage | couvert | enregistrement manuel (décidé le 2026-11-16) |
| `list_camera_events` | camera_fns.rs | compte | `detections_list` | lecture | couvert |  |
| `get_detector` | camera_fns.rs | admin | `config_get(detector)` | lecture | couvert |  |
| `set_detector` | camera_fns.rs | admin | `config_upsert(detector)` | configuration | couvert |  |
| `list_dashboards_admin` | dashboard_fns.rs | admin | `config_list(dashboard)` | lecture | couvert |  |
| `save_dashboard` | dashboard_fns.rs | admin | `config_upsert(dashboard)` | configuration | couvert |  |
| `delete_dashboard` | dashboard_fns.rs | admin | `config_delete(dashboard)` | configuration | couvert | `confirm` et `dry_run` |
| `list_widgets` | dashboard_fns.rs | admin | `dashboard_get` | lecture | couvert |  |
| `widgets_page` | dashboard_fns.rs | admin | `dashboard_get` | lecture | couvert |  |
| `save_widget` | dashboard_fns.rs | admin | `widget_upsert` | configuration | couvert |  |
| `delete_widget` | dashboard_fns.rs | admin | `widget_delete` | configuration | couvert | `confirm` et `dry_run` |
| `move_widget` | dashboard_fns.rs | admin | `dashboard_set_layout` | configuration | couvert |  |
| `send_device_command` | device_fns.rs | compte | `device_command` | pilotage | couvert | appareil actif seulement |
| `set_permit_join` | device_fns.rs | admin | `zigbee_pairing` | configuration | couvert | ouvre le réseau Zigbee à de nouveaux appareils (décidé le 2026-11-16) |
| `permit_join_state` | device_fns.rs | compte | `system_status` | lecture | couvert | champ `zigbee_pairing_open` |
| `generate_device_dashboard` | device_fns.rs | admin | `config_upsert(dashboard)` | configuration | couvert | génération du tableau Grafana d'un appareil (décidé le 2026-11-16) |
| `list_devices_admin` | devices_admin.rs | admin | `devices_list` | lecture | couvert |  |
| `save_device_admin` | devices_admin.rs | admin | `config_upsert(device)` | configuration | couvert |  |
| `delete_device_admin` | devices_admin.rs | admin | `config_delete(device)` | configuration | couvert | `confirm` et `dry_run` : supprime aussi ses automations |
| `device_choices` | devices_admin.rs | admin | `config_schema` | lecture | couvert | choix des formulaires = schéma |
| `list_device_groups` | devices_admin.rs | admin | `config_list(device_group)` | lecture | couvert |  |
| `save_device_group` | devices_admin.rs | admin | `config_upsert(device_group)` | configuration | couvert |  |
| `delete_device_group` | devices_admin.rs | admin | `config_delete(device_group)` | configuration | couvert | `confirm` et `dry_run` |
| `list_device_labels` | devices_admin.rs | admin | `config_list(device_label)` | lecture | couvert |  |
| `save_device_label` | devices_admin.rs | admin | `config_upsert(device_label)` | configuration | couvert |  |
| `delete_device_label` | devices_admin.rs | admin | `config_delete(device_label)` | configuration | couvert | `confirm` et `dry_run` |
| `devices_page` | devices_pages.rs | compte | `devices_list` | lecture | couvert |  |
| `device_detail` | devices_pages.rs | compte | `device_get` | lecture | couvert | et `device_history` |
| `energy_page` | energy.rs | compte | `energy_summary` | lecture | couvert |  |
| `list_logs` | logs_admin.rs | admin | `logs_query` | lecture | couvert |  |
| `compose_form` | messaging.rs | admin | — | — | exclu | messagerie exclue (décision du 2026-11-13) |
| `send_message` | messaging.rs | admin | — | — | exclu | messagerie exclue (décision du 2026-11-13) ; `notify_test` couvre l'essai des canaux |
| `sent_messages` | messaging.rs | admin | — | — | exclu | messagerie exclue (décision du 2026-11-13) |
| `inbox_messages` | messaging.rs | compte | — | — | exclu | messagerie exclue (décision du 2026-11-13) |
| `message_detail` | messaging.rs | compte | — | — | exclu | messagerie exclue (décision du 2026-11-13) |
| `list_mqtt_sources` | mqtt_sources.rs | admin | `config_list(mqtt_source)` | lecture | couvert |  |
| `save_mqtt_source` | mqtt_sources.rs | admin | `config_upsert(mqtt_source)` | configuration | couvert | avec code de traitement Rhai : portée `administration` (voir constats) |
| `delete_mqtt_source` | mqtt_sources.rs | admin | `config_delete(mqtt_source)` | configuration | couvert | `confirm` et `dry_run` |
| `list_prometheus_rules` | rules_admin.rs | admin | `config_list(prometheus_rule)` | lecture | couvert |  |
| `save_prometheus_rule` | rules_admin.rs | admin | `config_upsert(prometheus_rule)` | configuration | couvert |  |
| `delete_prometheus_rule` | rules_admin.rs | admin | `config_delete(prometheus_rule)` | configuration | couvert | `confirm` et `dry_run` |
| `list_grafana_rules` | rules_admin.rs | admin | `config_list(grafana_rule)` | lecture | couvert |  |
| `save_grafana_rule` | rules_admin.rs | admin | `config_upsert(grafana_rule)` | configuration | couvert |  |
| `delete_grafana_rule` | rules_admin.rs | admin | `config_delete(grafana_rule)` | configuration | couvert | `confirm` et `dry_run` |
| `sync_rules` | rules_admin.rs | admin | `config_upsert(prometheus_rule / grafana_rule)` | configuration | couvert | écriture des fichiers de règles automatique après l'enregistrement (décidé le 2026-11-16) |
| `list_scenes` | scenes.rs | compte | `scenes_list` | lecture | couvert |  |
| `activate_scene` | scenes.rs | compte | `scene_run` | pilotage | couvert |  |
| `list_scenes_admin` | scenes_admin.rs | admin | `config_list(scene)` | lecture | couvert |  |
| `save_scene` | scenes_admin.rs | admin | `config_upsert(scene)` | configuration | couvert |  |
| `delete_scene` | scenes_admin.rs | admin | `config_delete(scene)` | configuration | couvert | `confirm` et `dry_run` |
| `dry_run_script` | scripts.rs | admin | `script_check / automation_test` | administration | couvert | essai à blanc : aucun effet |
| `whoami` | server_fns.rs | session | — | — | exclu | session web ; l'assistant n'a pas de compte |
| `layout_data` | server_fns.rs | session | — | — | exclu | session web (barre de navigation du compte) |
| `logout` | server_fns.rs | session | — | — | exclu | session web |
| `login` | server_fns.rs | session | — | — | exclu | session web : les jetons remplacent la connexion |
| `change_password` | server_fns.rs | session | — | — | exclu | comptes et mots de passe hors périmètre |
| `list_locations` | server_fns.rs | admin | `config_list(location)` | lecture | couvert |  |
| `save_location` | server_fns.rs | admin | `config_upsert(location)` | configuration | couvert |  |
| `delete_location` | server_fns.rs | admin | `config_delete(location)` | configuration | couvert | `confirm` et `dry_run` |
| `list_navbar` | server_fns.rs | admin | `config_list(navbar)` | lecture | couvert |  |
| `save_navbar` | server_fns.rs | admin | `navbar_set` | configuration | couvert |  |
| `delete_navbar` | server_fns.rs | admin | `navbar_set` | configuration | couvert | `confirm` et `dry_run` |
| `list_settings` | server_fns.rs | admin | `settings_list` | administration | couvert | secrets : « défini » / « non défini » seulement |
| `save_setting` | server_fns.rs | admin | `setting_set` | administration | couvert | secrets en écriture seule |
| `list_users` | server_fns.rs | admin | — | — | exclu | comptes hors périmètre |
| `save_user` | server_fns.rs | admin | — | — | exclu | comptes hors périmètre |
| `delete_user` | server_fns.rs | admin | — | — | exclu | comptes hors périmètre |
| `broadcast_notification` | server_fns.rs | admin | `notify_broadcast` | pilotage | couvert | bandeau à tous les comptes connectés (décidé le 2026-11-16) |
| `inject_device_state` | server_fns.rs | admin | — | — | exclu | fabrique l'état d'un appareil (outil de test) : à ne pas offrir à un assistant |
| `dashboard_page` | server_fns.rs | session | `dashboard_get` | lecture | couvert | tableau de bord du compte connecté : à définir pour un jeton |
| `create_dashboard` | server_fns.rs | session | `config_upsert(dashboard)` | configuration | couvert | tableau de bord personnel : à définir pour un jeton |
| `list_weather_stations` | weather_admin.rs | admin | `weather_current / config_list(weather_station)` | lecture | couvert |  |
| `save_weather_station` | weather_admin.rs | admin | `config_upsert(weather_station)` | configuration | couvert |  |
| `delete_weather_station` | weather_admin.rs | admin | `config_delete(weather_station)` | configuration | couvert | `confirm` et `dry_run` |

## Les 12 fonctions hors catalogue : décisions du 2026-11-16 (validées par l'utilisateur)
- `sync_cameras` : outil `cameras_sync`, portée administration
- `sync_camera_onvif` : outil `cameras_sync`, portée administration
- `add_camera_by_ip` : exclu (interface seule)
- `camera_night` : outil `camera_night`, portée pilotage
- `list_snapshots` : outil `camera_snapshot(list)`, portée lecture
- `delete_snapshot` : exclu (interface seule)
- `delete_recording` : exclu (interface seule)
- `set_manual_record` : outil `camera_record`, portée pilotage
- `set_permit_join` : outil `zigbee_pairing`, portée configuration
- `generate_device_dashboard` : outil `config_upsert(dashboard)`, portée configuration
- `sync_rules` : outil `config_upsert(prometheus_rule / grafana_rule)`, portée configuration
- `broadcast_notification` : outil `notify_broadcast`, portée pilotage

## Constats qui modifient la feuille de route
1. **Le script Rhai passe par la porte de derrière.** `save_automation` (action « script ») et `save_mqtt_source` (code de traitement) acceptent du code Rhai. Avec un `config_upsert` en portée `configuration`, un jeton `configuration` écrirait des scripts, ce qui contourne le choix « scripts = `administration` » du 2026-11-13. **Règle proposée** : tout `config_upsert` dont la charge contient du script exige la portée `administration`.
2. **La lecture de configuration demande `admin` aujourd'hui.** Les 75 fonctions réservées aux administrateurs, listes de configuration comprises, exigent ce rôle. Pour MCP, `config_list` et `config_get` en portée `lecture` (comme `automations_list`) permettent un jeton de diagnostic sans droit de modification ; les secrets restent masqués. **Validé le 2026-11-16.**
3. **L'assistant intégré (R12) n'a pas de jeton** : il agit au nom de la session. Le service commun doit donc recevoir un **acteur** (`Jeton{portées}` ou `Session{compte}`), pas seulement un compte. À prévoir dès M1 pour ne pas refaire l'extraction.
4. **`dry_run` sur le pilotage.** R12 exige un essai à blanc pour toute action ; `ROADMAP-mcp.md` ne le prévoit que pour les suppressions et la configuration. À ajouter à `device_command` et `scene_run` (M3).
5. **Pas de retour arrière.** `dominet dump-context` exporte la configuration mais rien ne la réimporte : l'outil `rollback` de R12 suppose une restauration fiable, absente du catalogue de R11.
6. **Messages d'erreur.** Les fonctions serveur répondent « erreur interne » ou « accès refusé » : correct pour un navigateur, pauvre pour un assistant. Les validations du domaine renvoient déjà une erreur **par champ** ; les outils doivent la transmettre telle quelle.
7. **Délai de 30 s** sur tout le routeur : suffisant pour des outils qui répondent en millisecondes ; à surveiller pour `automation_test` et `logs_query`.

## Livré dans ce lot
Crate `dominet-mcp` (outil `system_status`, authentification `Bearer` provisoire `MCP_SPIKE_TOKEN`, désactivée par défaut `MCP_ENABLED=false`), montage par `serve(.., dominet_mcp::routes)`, tests de bout en bout avec un vrai client MCP (`cargo test -p dominet-mcp`), garde-fou de couverture, ADR 0058 et 0059. Voir [Serveur MCP](mcp.fr.md) pour l'essai avec Claude Code.
