# Pièges et leçons

Tous les pièges rencontrés pendant la construction de l'application, par domaine. Chaque ligne dit ce qu'on voit, pourquoi cela arrive, ce qui le corrige et, quand il existe, ce qui le garde désormais. À lire avant de reconstruire ou de porter une partie : aucun n'est évident à l'avance et la plupart ont été trouvés par un test, une construction de l'image ou la machine réelle, pas en lisant le code.

## 1. Construction et chaîne d'outils

| Symptôme | Cause | Remède / garde-fou |
|---|---|---|
| Boutons inertes, aucun message dans le navigateur | Le serveur et le WASM ne sont pas compilés pareil. `cargo leptos` ajoute `--cfg erase_components` et `LEPTOS_OUTPUT_NAME` ; un `cargo build` ou `cargo test` ordinaire non, donc le HTML du serveur ne correspond plus au WASM et l'hydratation échoue | Les deux réglages sont dans `.cargo/config.toml` pour toutes les compilations (ne pas les retirer). `ui.rs` vérifie le nom du WASM et les marqueurs d'hydratation ; l'E2E sur l'image est le test de référence |
| Échecs d'E2E sans queue ni tête | WASM ou image périmés : l'interface ou l'image n'a pas été reconstruite | Toujours `cargo leptos build --release` puis `cargo build -p dominet` avant l'E2E ; `make e2e-image` ne reconstruit pas, donc `make image-smoke e2e-image` |
| `make ci` est vert, l'image Docker ne se construit pas | Le Dockerfile ne copie que `crates/` et `.sqlx/` ; un `include_str!` hors de `crates/` compile en local | Rien ne s'inclut depuis l'extérieur de `crates/` ; `make ci-full` avant un jalon ou quand le Dockerfile ou une dépendance native change |
| L'image ne sait pas lier ONNX Runtime | La bibliothèque C++ précompilée exige `g++` à la construction, `libstdc++6` à l'exécution et une glibc 2.38 ou plus | Reconstruire l'image dès qu'on ajoute une dépendance native |
| Le serveur panique au démarrage quand `REDUCTSTORE_URL` est défini, seulement dans l'image | L'image n'a pas de certificats racine ; le client HTTP rustls panique à sa construction | L'image installe `ca-certificates` ; la construction du client ne panique plus |
| La construction de l'image échoue au hasard | `wasm-bindgen` et `wasm-opt` sont téléchargés depuis un CDN | Le Dockerfile réessaie quatre fois ; sinon relancer |
| Risque d'un binaire inutilisable sur la cible | `-C target-cpu=native` lie le binaire au processeur de la machine de construction (la cible est un Skylake sans AVX-512) | Ne jamais publier avec `target-cpu=native` ; base `x86-64-v2` (ADR 0001) |
| `sqlx` refuse une requête | `sqlx` 0.9 exige `AssertSqlSafe` pour tout SQL construit dynamiquement | Envelopper le SQL dynamique volontairement, ne jamais concaténer une saisie |
| La compilation hors ligne échoue après l'ajout d'une requête | Les métadonnées `.sqlx/` sont périmées | Toute nouvelle `sqlx::query!` est suivie de `make sqlx-prepare` ; `make sqlx-check` le garde |
| Clippy signale des noms propres dans les commentaires | `clippy::doc_markdown` | Déclarer les noms propres dans `clippy.toml` |
| Les dépendances dérivent vers des avis de sécurité | Une crate transitive reçoit une entrée RUSTSEC (une crate de messagerie en a eu une) | `cargo update -p <crate>` ; `make ci` lance `cargo deny` et `cargo audit` |

## 2. Interface web (Leptos, rendu serveur)

- **Page figée après un démarrage à froid** (Leptos 0.8.23) : des rendus concurrents peuvent ne jamais se terminer, dans 1 à 5 % des démarrages à froid. Les `panics` « reactive value disposed » du journal en sont la conséquence, pas la cause. Garde-fou : `web/render_guard.rs` abandonne et relance un `GET` sans réponse en 5 s (3 essais, puis 503) ; reproducteur `tools/stress-cold-render.sh` (ADR 0021).
- Lire une ressource sous `Transition`, sinon le rendu serveur ne l'attend pas. Rendre les routes en `SsrMode::Async`.
- Pas de `style=` ni de `on…=` en ligne (politique de sécurité du contenu). `prop:value` n'est pas rendu côté serveur : utiliser `value=` pour l'état initial et `disabled=` plutôt que `prop:disabled`.
- Pas de `StoredValue::new_local` dans un composant rendu côté serveur (un `panic` `SendWrapper` à la libération fige la page).
- Pas de ressource dont la source dépend d'une autre ressource (chargements dans le désordre à l'hydratation, tableau périmé).
- Pas d'accolades dans un fichier `.ftl` (`panic` au chargement des traductions).
- Lire les champs de formulaire par `NodeRef` et la valeur de l'entrée, pas par des signaux : le remplissage automatique du navigateur remplit les champs sans événement `input`.
- Plusieurs applications Leptos rendues en parallèle dans un même processus de test se perturbent : `ui.rs` les exécute une par une sous verrou.
- Les liens de la barre de navigation doivent être chargés par le navigateur, pas par le routeur de l'interface, quand la cible est servie par le serveur (`/docs/`, `/grafana/`) : le routeur répond « page introuvable ».
- Un champ obligatoire de formulaire qui ne sert pas dans tous les cas bloque la création (un widget Grafana était impossible à créer parce que « appareil / scène / caméra » étaient tous obligatoires).
- Un onglet resté ouvert pendant une mise à jour garde l'ancien front et peut afficher une pastille sans sa liste. `/pkg/` est servi `no-cache` avec un ETag ; un rechargement forcé suffit.
- Le WASM de développement fait 48 Mo (boutons inertes 10 à 30 s) ; utiliser une compilation release pour les démonstrations.

## 3. Authentification et sessions

- `flush()` seul ne renouvelle pas l'identifiant de session : utiliser `flush` puis `cycle_id`, sinon la session est fixée.
- Une session volée pouvait deviner le mot de passe sans limite : appliquer le même blocage que la connexion.
- Deux administrateurs qui se rétrogradent en même temps ne laissaient aucun administrateur : un verrou consultatif le sérialise (test de course).
- Jetons MCP : hachés, à portées, date de fin obligatoire, révocation immédiate, limite de débit ; les portées sont appliquées en un seul endroit.

## 4. Base de données et tests

- Créer des bases jetables (`#[sqlx::test]`) force un checkpoint ; à 20 fils de test, `PoolTimedOut` apparaît environ une fois sur trois. Les tests sont bornés à 8 fils (`TEST_THREADS`). Le même `PoolTimedOut` apparaît sur tous les tests de base quand la base de développement n'est tout simplement pas démarrée.
- Courses trouvées seulement par des tests de course voulus : un tableau par défaut lu avant son verrou, deux insertions simultanées de widgets bloquées sur une contrainte d'exclusion (prendre d'abord le verrou de ligne).
- `pg_dump` 17 contre un serveur PostgreSQL 16 écrit `SET transaction_timeout`, inconnu du serveur : retirer cette ligne en restaurant sans l'outil.
- Une tâche longue au démarrage (tâches de reprise) doit tourner en arrière-plan ; attendre la base avant de lancer le serveur HTTP faisait échouer le test de fumée de l'image quand la base est injoignable.
- Un test ne doit pas supposer un courtier vide ni un moteur déjà chargé : les moteurs relisent leur liste en tâche de fond, les tests répètent donc l'action jusqu'à la première réaction.

## 5. MQTT et Zigbee

| Symptôme | Cause | Remède |
|---|---|---|
| Une touche de télécommande est perdue | L'ingestion ne gardait que le dernier état d'un appareil reçu dans la même seconde ; une télécommande envoie `action: 1_single` puis `action: ""` à quelques millisecondes d'écart | Les automations voient aussi les états regroupés (ADR 0032) |
| Appareil absent après la création d'une source MQTT | Un message **retenu** est arrivé avant que « Appareil automatique » soit coché | Le cocher à la création ; un appareil à message retenu n'apparaît sinon qu'au réabonnement |
| Test de débit instable sous charge | Mosquitto **jette** les messages au-delà de 1 000 en attente par client | `max_queued_messages 100000` en développement et en production |
| L'accès anonyme fuit vers le listener sécurisé | Sans `per_listener_settings`, `allow_anonymous true` s'applique partout | `per_listener_settings true` ; un seul listener avec mot de passe, comptes par client (ADR 0057) |
| Le bouton d'appairage ne fait rien | Zigbee2MQTT 2.x refuse `{"value": true}` (« Invalid payload ») | Envoyer `{"time": 254}` |
| Un appareil déjà appairé ailleurs ne s'appaire pas | Il se souvient de son ancien réseau | Le réinitialiser d'abord |
| Renommer un appareil casse ses commandes | Le `friendly_name` d'un appareil est aussi son topic MQTT | Renommer depuis l'administration, en sachant que le topic change |
| Arrêt d'un volet en mouvement impossible | Le relais n'a ni clé `moving` ni état `STOP` : son état dérive de la position et il publie environ un message par seconde en mouvement | Utiliser l'âge du dernier message (`age_secs` dans les scripts). Ne jamais écrire un script d'après le nom d'un plugin |
| Un message de refus reste après le retour du courtier | L'avis ne s'effaçait que par une action de l'utilisateur | Il s'efface dès que l'appareil republie un état |
| Les tests voient d'anciens messages Zigbee | Le courtier de développement garde des messages retenus | Préfixe de topics propre à chaque test et `Z2M_BASE_TOPIC` détourné |

## 6. Vidéo et détection

- Le **décodage logiciel** dans le conteneur prenait 85 à 90 % d'un cœur : le conteneur n'avait ni `/dev/dri`, ni groupe `render`, ni pilote. Correction : VAAPI (`make prod-video`), mesuré à 36 %.
- Le repli VAAPI était déclenché trop vite par une caméra hors ligne. Seule la machine réelle l'a révélé.
- ReductStore 1.21 et plus refuse un `RS_API_TOKEN` vide ; son image n'a ni `wget` ni `curl` (contrôle de santé par `bash` et `/dev/tcp`).
- MediaMTX : le schéma de configuration change entre versions ; le revalider à chaque montée. Le port ICE (8189, UDP et TCP) est fixe : une seule pile à la fois.
- Le mot de passe d'une caméra apparaît en clair dans la réponse ONVIF `GetStreamUri` : le masquer partout.
- Un seul client TCP à la fois sur l'onduleur solaire : une lecture de diagnostic échoue pendant que le rôle publisher tourne.
- Les fixtures ONVIF sont reconstituées à partir de captures ; les confronter aux vraies caméras avec le test ignoré `real_onvif`.
- Un script d'endurance qui attend les segments indexés mais pas les segments archivés fait une course.

## 7. Logique métier

- `float("1_000")` est valide en Python et refusé par Rust : l'analyseur des règles d'alerte a dû l'accepter (trouvé en comparant à l'implémentation de référence).
- Heures affichées en UTC (`…Z`) dans la liste des automations : le calcul est juste (les événements solaires sont des instants absolus, les plannings passent par `TZ`, défaut `Europe/Paris`, avec gestion des heures ambiguës et inexistantes) mais l'affichage devrait être local.
- Bilan énergétique : troncature plutôt qu'arrondi (`int(1.005 × 1000)`), nuits de changement d'heure, bordures de jour local, remise à zéro du compteur, jours sans donnée. Ces cas sont couverts par les fixtures de référence de `tests/fixtures/energy`.
- Ne jamais recopier une coordonnée d'un fichier d'exemple : la météo et les heures solaires d'une installation entière étaient fausses parce que les coordonnées d'exemple avaient été gardées.

## 8. Observabilité (Prometheus, Grafana)

- Grafana exige une adresse d'expédition SMTP avec un point et un destinataire non vide, sinon il redémarre en boucle. Un volume monté en `:ro` empêche les montages imbriqués.
- Les règles d'alerte Grafana avec `NoData` réglé sur « sans données » se déclenchent sans fin : « aucun résultat » veut dire « tout va bien » pour ces règles. Mettre `OK` par défaut ; corriger à la main les installations existantes.
- 288 courriels en échec en 12 heures : un mot de passe SMTP périmé (services non recréés après la modification de `.env`) et un destinataire écrit `=…`.
- Compose interpole aussi les variables des profils inactifs : `prod-guard` les vérifie.
- Prometheus doit charger tous les groupes de règles ; un compte tiré du fichier de règles est le bon contrôle (`tools/check-observability.sh`).
- Les alertes de plateforme de `/alerts` lisent `GET /api/v1/alerts` de Prometheus (état `firing`) et l'API du gestionnaire d'alertes de Grafana en authentification basique ; le hub est en processus, donc le relevé tourne dans le rôle web (ADR 0061).
- Le tableau Énergie trouve les appareils par clés d'état (`pac`, `qdy`, `PAPP`, `power_ab`) ; ces clés doivent être **suivies** dans l'historique de l'appareil, ce que `dominet seed-energy` fait désormais.

## 9. Production et exploitation

| Symptôme | Cause | Remède |
|---|---|---|
| `prod-deploy` s'arrête avant toute compilation | Compose 5.6 délègue à `buildx bake`, qui refuse `network: host` dans `build` | Retirer la ligne |
| Firefox refuse le certificat, Brave l'accepte | Firefox a son propre magasin de certificats | Importer l'autorité à la main (ou `security.enterprise_roots.enabled`) ; `libnss3-tools` pour `mkcert -install` |
| Courriels envoyés à `=adresse` | Double `=` dans `.env` (`NOM==valeur`) | `prod-guard` le signale |
| `.env` modifié, rien ne change | Le service n'a pas été recréé | Recréer les services après toute modification de `.env` |
| `make prod-logs` bloque avec `grep` | Il suit en continu | `N=100` pour les dernières lignes sans suivi |
| Disque plein sur la machine de production | Un `target/` de 370 Go et un cache de construction de 73 Go laissés dessus | Ne jamais compiler sur la machine de production |
| Le push échoue à cause du hook de 7 minutes | GitHub coupe la connexion pendant le hook | `ssh -o ServerAliveInterval=30` |
| Le hook `pre-push` échoue sur les tests MQTT | Il utilise la pile de développement, qui était arrêtée | `make dev-up` d'abord |
| Le bilan ne montrait plus les sauvegardes PostgreSQL | Il gardait les 4 dernières lignes, toutes sortes confondues | Deux par sorte |
| Bouton « Se connecter » inerte dans un navigateur | L'interface n'a pas démarré dans l'onglet ; cause jamais identifiée | Vider le cache ou désactiver les protections du navigateur |

## 10. Tests de bout en bout

- Un clic avant l'hydratation « réussit » sans effet : attendre l'indicateur temps réel ou répéter le clic jusqu'à l'ouverture du formulaire (rencontré quatre fois).
- Le serveur SMTP simulé est partagé par les scripts : filtrer sur son propre sujet. Le serveur d'E2E retient le dernier état des appareils : donner à chaque script son numéro d'appareil.
- Deux scripts lancés ensemble peuvent se perturber (`alerts` avec `platform_alerts` : le faux Prometheus est compté dans la pastille). Les lancer séparément.
- Deux étapes instables jamais expliquées, jamais reproduites : la largeur de la barre mémoire dans `system` et le diagnostic `publish solarmax`. Les noter, ne pas les cacher.
- Lancer `make ci` souvent : une casse des tests d'interface (traductions tronquées) n'a été vue qu'en fin de lot.

## 11. MCP et assistant local

- Un script passe par la porte de derrière : `save_automation` et `save_mqtt_source` acceptent du code, donc un jeton `configuration` pourrait écrire des scripts. Règle : tout `config_upsert` dont la charge contient un script exige la portée `administration`.
- Les fonctions serveur répondaient « erreur interne » ou « accès refusé », correct pour un navigateur, pauvre pour un assistant : les outils transmettent telle quelle l'erreur de validation par champ.
- L'audit est écrit **avant** l'exécution et l'appel est refusé si l'audit ne peut pas être écrit.
- `dry_run` sur toute action, y compris les commandes d'appareil et les scènes.
- L'assistant reçoit un **acteur** (portées d'un jeton ou session d'un compte), pas seulement un compte.
- **Le modèle n'exécute jamais un changement.** Il propose une carte d'action ; l'utilisateur confirme d'un clic ; un instantané est pris avant et le changement peut être annulé.
- Un petit modèle (Qwen3-8B) demande parfois la permission de continuer (« voulez-vous continuer ? », « nous devons d'abord… ») au lieu d'appeler l'outil : un garde-fou le relance une fois. Il ne sait pas modifier ni renommer de façon fiable un élément **existant** : le faire depuis l'administration. Le modèle 14B mesuré n'était pas meilleur (96 % contre 98 %), un seul modèle a donc été gardé.
- Un `id` entier placé dans les données au lieu du paramètre `id` doit être déplacé par l'outil.
- Tout ce qui est lu par les outils est une donnée, jamais une instruction (essai d'injection avec le vrai modèle).
- Jeu d'évaluation de 51 demandes : 49 réussies, aucune ne produit une mauvaise carte (les deux échecs ne posent aucune question et ne produisent aucune carte).

## 12. Méthode

- La machine réelle trouve ce qu'aucun test de développement ne voit : décodage sur le processeur, coordonnées, certificats, courrier, disque. Prévoir une répétition sur du vrai matériel.
- Compter ce qu'on livre : mettre le nombre dans le test (cas comparés, scènes, règles) pour qu'une réduction silencieuse se voie.
- Une fonction peu fiable avec le modèle choisi doit être retirée plutôt que cachée : le renommage par l'assistant a été abandonné.
- Une décision non évidente reçoit un ADR, avec les options écartées.
