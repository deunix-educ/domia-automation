# Tests

637 tests Rust + 22 scripts E2E navigateur (604 vérifications). Tout se lance en local : il n'y a plus de CI automatique
([ADR 0015](adr/0015-local-gate.md)).

| Commande | Contenu | Durée |
|---|---|---|
| `make ci` | formatage, clippy (serveur **et** WASM), tests Rust, `cargo deny`, `cargo audit`, métadonnées `sqlx` | ≈ 2 min à chaud |
| `make e2e` | E2E navigateur sur `target/debug/dominet` (après `make front`) | ≈ 2,5 min |
| `make ci-full` | `make ci` + image Docker (build, test de fumée) + **E2E sur l'image** | selon le cache Docker |
| `tools/stress-cold-render.sh [image] [n] [concurrence] [page]` | démarrages à froid en boucle avec requêtes concurrentes : mesure le rendu figé de Leptos que le garde-fou contourne ([ADR 0021](adr/0021-page-render-guard.md)) ; attendu : 0 anomalie | ≈ 3 s par démarrage |
| `make hooks` | garde-fou `pre-push` : `make ci` sur une copie propre du commit poussé | |

## Tests Rust (`cargo test --workspace`)

Les tests de base de données utilisent `#[sqlx::test]` : une base isolée par test, créée sur le PostgreSQL de la stack de dev (`make dev-up`).

| Niveau | Où | Tests | Ce qui est couvert |
|---|---|---|---|
| Unitaires | `dominet-domain` | 97 | types validés, politique de mot de passe, blocage, motifs de refus par champ, grille et widgets, règles d'affichage (présentation automatique, unités, adresse Grafana), appareils (type déduit des `exposes`, lignes d'historique, noms et filtres de topics, libellés connus) |
| | `dominet-proto` | 9 | format des messages, sujets et DTO (contrat serveur / navigateur) |
| | `dominet-server` (lib) | 85 | configuration, erreurs, chiffrement des secrets, hub WebSocket, CSRF, statistiques système (conversion, capteur de température, collecteur), garde-fou de rendu (relance, 503, `POST` intact) |
| | `dominet-automation` (lib) | 20 | aiguillage des topics, regroupement des rafales, **hôte Rhai** (API, effets dans l'ordre, tout ou rien, **scripts hostiles** : boucles, récursion, chaînes / tableaux / tables géants, `pad`, `eval`, imbrication, effets sans fin ; limite de durée seule ; refus d'un autre langage) |
| | `dominet-mqtt` (lib) | 12 | correspondance de topics, décodage des charges utiles, différence d'abonnements, topics de publication, mot de passe masqué, échec immédiat sans broker |
| | `dominet-integrations` (lib) | 9 | protocole SolarMax : requêtes identiques octet pour octet à celles de référence, réponses, mises à l'échelle, arrondi demi-pair, grandeurs dérivées (vecteurs de référence figés) ; requête et normalisation Open-Meteo |
| Publishers | `dominet-integrations/tests` | 10 | client SolarMax contre un onduleur **simulé** (cycle, alarmes, modèle inconnu, silence, bruit, reprise, port fermé) ; météo contre une API simulée (une station en échec n'arrête pas les autres, pause = plus court intervalle), supervision d'une tâche qui panique |
| | `dominet-web` (lib) | 21 | adresses de retour (`next`), reconnexion, abonnements comptés, CSP, icônes Lucide, éditeur de couples du formulaire |
| Dépôts et schéma | `dominet-store/tests` | 83 | schéma (18, dont l'exclusion de chevauchement des widgets), appareils, historique, images, sources et stations (14 : contraintes, sémantique de l'écoute MQTT, clé étrangère des widgets), dépôts (27, dont la règle du dernier administrateur, les courses sur le tableau par défaut et sur le placement des widgets), sessions et tentatives (12) |
| API et services | `dominet-server/tests` | 77 | authentification et blocage (20), comptes (6), réglages et secrets (17), santé (6), erreurs de base (4), contrat de `/api/system-stats/` (4), `seed-labels` (1), rôle `mqtt` de bout en bout sur le broker de dev (2), **moteur d'automations de bout en bout** sur le broker de dev (3 : la télécommande de la production déplace le volet, le coucher / lever du soleil de la production active ses scènes via le planificateur, topic sans source, scène, désactivation à chaud), **règle canicule de la production de bout en bout** sur le broker de dev (1 : alerte, bandeau, pastille, cooldown, résolution), **plugins Rhai de la production et code de source de bout en bout** sur le broker de dev (1), **activation des scènes** sur le broker de dev (3 : ordre des commandes, échec qui n'arrête pas la suite, les 3 scènes de la production : 6 / 4 / 4 commandes sur les bons topics) |
| Rejeu de session | `dominet-automation/tests/replay.rs` | 4 | session de 64 messages rejouée message par message, par paquets d'une seconde et deux fois de suite : mêmes appareils, états et historique que le résultat de référence ; rafale de 4 000 messages |
| Rejeu de sources | `dominet-automation/tests/replay_sources.rs` | 1 | session de 36 éléments (sources, appareils automatiques, réactivation, marquage « vu », préfixes, sources inactives, charges non objet) : mêmes appareils, états et historique que le résultat de référence — un seul écart, documenté (premier message d'un appareil automatique) |
| Débit | `dominet-server/tests/throughput.rs` | 1 | 2 000 messages/s pendant 5 s, QoS 1, de bout en bout (broker de dev → base → diffusion) : dernier état de 50 appareils conservé, retard final < 5 s (mesuré ≈ 0,1 s) |
| Sources MQTT | `dominet-automation/tests/sources.rs` | 7 | appareil automatique (nom sans préfixe, type, état remplacé, réactivation), historique et archive des clés suivies, sources inactives, charges non objet, appareils marqués vus |
| Scènes, automations, alertes, énergie | `dominet-store/tests/automation.rs` | 10 | scènes (ordre des actions, remplacement transactionnel, nom unique, appareil inexistant, clé étrangère du widget), **scénarios de la production** (3 scènes de 6 / 4 / 4 actions, 7 automations relues à l'identique), compteurs et journal d'exécution, travail du planificateur, 9 événements solaires, alertes (en cours / résolue / acquittée), bilan d'énergie |
| Moteur d'automations | `dominet-automation/tests` (`matching`, `engine`, `hooks`) | 3 + 7 + 2 | **396 cas comparés à des valeurs de référence figées** (correspondance de topics, condition, filtre de payload : les fichiers de référence) ; déclencheurs `device_state` / `mqtt_message`, condition, actions, journal et compteurs, garde-fou 30/min, file bornée, **automations de la production** ; messages et états regroupés annoncés au moteur |
| Scripts dans le moteur | `dominet-automation/tests/engine.rs` | (dans les 10) | script qui voit déclencheur, appareils et effets dans l'ordre ; script en erreur ou hostile : rien d'appliqué, journalisé, le moteur continue ; **les deux plugins de la production portés en Rhai** ; code de source en sourdine après un échec |
| Énergie | `dominet-automation/tests/energy.rs` | 2 | **13 jours comparés à des valeurs de référence figées**`) ; longueur du jour local (23 / 24 / 25 h) |
| Acceptation R4 | `dominet-server/tests/acceptance_r4.rs` | 1 | **tout le `context.yaml` de la production à la fois** (29 appareils, 3 scènes, 7 automations dont les 2 plugins Rhai, la règle canicule) sur le Mosquitto de dev : télécommande, plugin, alerte et bandeau, coucher du soleil → six commandes de la scène, sans se gêner |
| Alertes | `dominet-domain/tests/alerts_oracle.rs`, `dominet-automation/tests/alerts.rs` | 1 + 4 | **2 030 évaluations de règle comparées à des valeurs de référence figées** ; **8 scénarios du cycle des alertes** (cooldown, acquittement qui sort du cooldown, valeur absente, texte, bascule rapide) comparés à des valeurs de référence **après chaque étape** sur base jetable ; textes de référence, redémarrage, règles éteintes, poignée |
| Relais Android | `dominet-domain/tests/android_fixtures.rs`, `dominet-store/tests/android.rs`, `dominet/tests/android_ws.rs`, `dominet/tests/android_oracle.rs` | 2 + 7 + 15 + 1 + 2 + E2E | trames tirées de l'application Android ; dépôts (jeton haché, accusés, délai) ; point d'accès sur un vrai serveur TCP (jeton, présence, remplacement, battement, SMS, appels, accusés, délai de 60 s, messagerie, alertes, droits) ; **21 sessions rejouées contre des échanges de référence figés** |
| Canaux d'alerte | `dominet/tests/alert_channels.rs` | 5 + E2E | le **vrai moteur d'alertes** déclenche : e-mail (expéditeur, destinataire, sujet `[DomiNet Alerte …]`, texte long), SMS (identifiants, texte court), push reçu par un téléphone connecté, canaux de la règle seulement, échec d'un canal sans effet sur les autres, canal non configuré ignoré, notification d'automation (SMS seulement si demandé) |
| Messagerie | `dominet/tests/messaging.rs`, `dominet-server` (unitaire `resolve_audience`) | 6 + 1 + E2E | e-mails vers un **faux SMTP** (expéditeur, sujet, texte), SMS confié à un téléphone qui parle comme l'application, statuts avant / après l'accusé, échecs avec leur raison, SMS sans accusé échoué à 60 s, évènements `message:<id>` jusqu'à la fin, droits d'abonnement, reprise après redémarrage |
| Planificateur | `dominet-domain/tests/schedule_oracle.rs`, `dominet-automation/tests` (`schedule`, `scheduler`) | 1 + 2 + 4 | **cron et soleil comparés à des valeurs de référence figées** : 119 suites de six passages d'un cron en `Europe/Paris` (nuits de changement d'heure, 29 février) identiques à `TzAwareCrontab`, 1 036 événements solaires à 13 s au plus d'`ephem` ; synchronisation avec les automations, passages à l'heure, pas de rattrapage, échecs, **coucher / lever du soleil de la production** |
| Archive ReductStore | `dominet-store/tests/reduct.rs` | 2 | enregistrements avec étiquettes numériques et horodatages distincts (bucket jetable du ReductStore de dev) ; serveur injoignable : jamais bloquant, enregistrements comptés |
| Publishers de bout en bout | `dominet-server/tests/publishers.rs` | 2 | API météo et onduleur simulés → publisher → Mosquitto de dev → sources MQTT → appareils ; sans station ni `SOLARMAX_HOST`, rien n'est demandé |
| Contexte | `dominet-server/tests/context.rs` | 1 | `dump-context` : appareils actifs triés par type puis nom, schéma d'état, YAML |
| Broker MQTT | `dominet-mqtt/tests` | 6 | Mosquitto de dev (aller-retour décodé, messages retenus, abonnements à chaud, arrêt) et **Mosquitto jetable** (broker apparu après le client ; coupure de 3 s : aucun message accepté perdu, publications refusées pendant la coupure) — Docker requis |
| Application complète | `dominet/tests/ui.rs` | 84 | rendu serveur FR/EN, gardes, connexion, mot de passe, CRUD des quatre pages, droits, secrets, CSP, cache, compression, simulateur d'état, jauges système, tableau de bord (invitation, placement par classes, HTML assaini, Grafana, sélecteur, isolation des comptes), **aucune donnée sensible dans les journaux** |
| WebSocket | `dominet/tests/ws.rs` | 15 | vrai serveur TCP et client : session, origine étrangère, limite, battements, coupure à la déconnexion, abonnements, dernier état, refus, indépendance des connexions |
| Traductions | `dominet-web/tests/i18n.rs` | 3 | mêmes clés en FR et EN, aucune clé utilisée absente, aucune clé morte |

Chemins critiques de R1 et leurs tests : **connexion et blocage** (`auth.rs`, `ui.rs`), **sessions** (révocation, fixation : `auth_store.rs`, `auth.rs`),
**droits administrateur** (`ui.rs`, chaque fonction serveur testée en anonyme et en compte ordinaire), **secrets** (jamais renvoyés : `settings.rs`, `ui.rs`),
**dernier administrateur** (`repos.rs`, `ui.rs` : course entre deux administrateurs, vérifiée en neutralisant le verrou), **WebSocket** (`ws.rs`),
**CSP et nonce** (`ui.rs`), **hydratation** (nom du WASM et marqueurs dans le HTML : `ui.rs`).

**Parallélisme.** `make test` borne les tests à 8 fils (`TEST_THREADS`) : chaque `#[sqlx::test]` crée une base, `CREATE DATABASE` force un checkpoint PostgreSQL, et à 20 fils sur une machine de 20 cœurs la file dépasse le délai de connexion de sqlx (`PoolTimedOut`, mesuré sur `android_ws` : échecs 1 fois sur 3 à 20 fils, aucun sur 6 essais à 4 fils).

## E2E navigateur (`tools/e2e.sh`)

Firefox headless piloté par Marionette (`tests/e2e/marionette.py`, sans dépendance Python externe). Le lanceur crée une **base jetable**, un profil
Firefox temporaire et un serveur, rejoue les scripts puis supprime tout (même en cas d'échec) :

| Script | Vérifie |
|---|---|
| `home.py` | rendu serveur, **hydratation WASM** (repli du menu), traductions à chaud, session |
| `r2.py` | **parcours R2 par l'interface seule (sans SQL)** : tableau créé depuis l'invitation, un widget de chaque type ajouté par l'administration, message et état d'appareil d'essai, suivi en direct, déplacement au pointeur, rechargement, refus d'un emplacement occupé |
| `dashboard_edit.py` | édition : déplacement au pointeur (persistant, emplacement occupé refusé), administration des widgets (champs selon le type, chevauchement, HTML assaini, suppression) et des tableaux |
| `dashboard.py` | tableau de bord : création depuis l'invitation, placement par classes, texte assaini, Grafana (iframe, refus, CSP), appareil suivi en direct, sélecteur sans rechargement, petit écran ; prépare ses widgets en SQL (`E2E_DB_SHELL`) |
| `auth.py` | gardes, remplissage automatique, échec puis succès, traduction, redirection ouverte refusée, changement de mot de passe, déconnexion |
| `layout.py` | barre latérale rendue par le serveur, lien actif, repli mémorisé, langue, 404, déconnexion |
| `locations.py`, `admin.py`, `users.py` | CRUD générique, navigation (la barre suit), réglages (secret jamais affiché), utilisateurs (protections) |
| `system.py` | jauges système : rendues vides par le serveur, remplies par le WebSocket sans recharger, largeur appliquée malgré la CSP, cohérence avec l'API, flux continu, infobulles FR/EN |
| `mqtt.py` | MQTT → base → temps réel → widget : découverte (type déduit), état qui remplace le précédent, historique, disponibilité, bruit ignoré, préfixe modifiable à chaud ; sur le Mosquitto de dev |
| `mqtt_sources.py` | page `/admin/mqtt-sources` : ajout (filtre invalide refusé), appareil créé par un message, état remplacé, charges non objet ignorées, désactivation et suppression à chaud |
| `devices.py` | administration (groupe, appareil, liste déroulante), `/automation/` (cartes, filtres), état publié sur le broker, commandes (bouton, curseur) reçues par le broker, hors ligne, appairage, page de l'appareil (topics, image téléversée, réduite puis supprimée, courbe, tableau Grafana) |
| `scenes.py` | scènes (R4) : administration (commandes ordonnées, appareil en liste, JSON refusé s'il est invalide, ordre inversé), page `/automation/scenes`, activation qui publie **dans l'ordre** sur le broker de dev, retour ✓ / ✗ (appareil inactif ignoré, la suite part), widget scène (liste déroulante, bouton), suppression |
| `automations.py` | automations (R4) : formulaire selon le type, JSON invalide refusé, état publié → commande publiée, topic sans source (filtre, condition), notification affichée, journal, garde-fou 30/min, planificateur (cron chaque minute, coucher du soleil, prochain passage), désactivation et suppression à chaud |
| (dans `automations.py`) | scripts Rhai : syntaxe refusée avec la ligne, code d'un autre langage refusé, essai à blanc (effets, journal, boucle infinie interrompue), exécution sur un message, boucle infinie pendant que le serveur répond, moteur toujours vivant ensuite |
| `alerts.py` | alertes (R4) : règle créée depuis l'administration (seuil illisible refusé, virgule décimale), état publié → alerte, bandeau et pastille en direct, page `/alerts` (filtres, acquitter, tout acquitter), alerte acquittée = hors cooldown, résolution, règle supprimée : les alertes restent |
| `energy.py` | énergie (R4) : historique de compteurs, **le vrai planificateur** calcule hier (mêmes Wh que le calcul de référence), page `/automation/energy` (SVG sans script, plages, tableau), API `/api/energy/`, état en direct (production, Linky, PAC, bilan net) sans recharger |
| `weather.py` | stations météo (administration, virgule décimale, latitude hors plage), relevé immédiat à la création contre une API **simulée**, onduleur simulé → appareils (source MQTT) visibles sur `/automation/`, désactivation = plus aucune requête ; diagnostics `publish solarmax --print-only` et `dump-context` |
| `android.py` | pages Android avec un **téléphone simulé** (`tools/sim_android.py`) : appareil ajouté (jeton montré une fois, haché en base), présence, SMS et appels en direct, SMS écrit dans le navigateur (en attente / envoyé / échec), jeton régénéré (l'ancien coupé en 4001), suppression |
| `messaging.py` | messagerie : formulaire refusé vide, envoi asynchrone, statuts en direct (e-mails vers un **SMTP simulé** `tools/sim-smtp.py`, SMS accusé à la main par le téléphone simulé), boîte de réception du destinataire sans le tableau des autres, pages réservées, échec sans téléphone avec sa raison |
| `mcp.py` | serveur MCP (R11 M1) : jeton créé dans `/admin/mcp` (formulaire refusé sans nom ni portée), **valeur montrée une seule fois**, appel réel de `/mcp` avec ce jeton (`system_status`), 401 sans jeton ou avec un jeton inconnu, révocation avec confirmation puis 401 dès l'appel suivant, journal d'audit sans la valeur, traduction à chaud |
| `mqtt_outage.py` | reprise après coupure : le Mosquitto de dev est **arrêté puis relancé** ; pendant la coupure une commande est refusée tout de suite avec un message, le serveur reste joignable ; ensuite, sans redémarrage, états et commandes reprennent et le message de refus s'efface |
| `realtime.py` | CSP appliquée (script sans nonce bloqué), WebSocket, diffusion, simulateur d'état d'appareil, reconnexion et réabonnement après redémarrage du serveur |

`tools/e2e.sh --image dominet:dev` (ou `make e2e-image`) rejoue le tout **sur l'image Docker** : c'est le seul test qui voit le binaire de production
(compilé en release) avec les fichiers WASM qui l'accompagnent. Un serveur et un WASM compilés différemment donnent une interface inerte (voir
`PROJECT_CONTEXT.md`, « Pièges connus ») ; seuls les tests qui hydratent vraiment la page dans un navigateur le détectent.

## Ajouter un test

- Une règle métier → test unitaire dans `dominet-domain`. Une requête SQL → test de dépôt (`dominet-store/tests`).
- Une fonction serveur ou une page → `dominet/tests/ui.rs` (toujours : cas anonyme, compte ordinaire, administrateur).
- Un comportement qui n'existe que dans le navigateur (hydratation, clic, WebSocket, CSP) → un script dans `tests/e2e/`, et l'ajouter à la liste de
  `tools/e2e.sh`. Les formulaires se testent aussi sans événement `input` (remplissage automatique).
