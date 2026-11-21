# Reconstruire de zéro

L'ordre dans lequel construire l'application, ce qu'il faut écrire en premier, quels pièges lire avant chaque étape et comment savoir qu'une étape est finie. On suppose que vous avez lu [Pièges et leçons](pitfalls.fr.md), [Mesures](measures.fr.md) et [Décisions par thème](decisions.fr.md) ; chaque étape ci-dessous renvoie aux parties utiles.

## Principes

- **Une phase à la fois, puis s'arrêter et vérifier.** Une phase se termine par sa définition de fini et une démonstration ; la suivante ne commence pas avant qu'elle soit acceptée.
- **Écrire d'abord les données de référence.** Sessions d'appareils enregistrées, résultats attendus des alertes et des plannings, vecteurs de protocole et réponses de caméras sont peu coûteux à produire tôt et chers à deviner plus tard. Les tests qui comparent à de tels fichiers ont attrapé la plupart des défauts subtils (voir la page des pièges, logique métier).
- **Un spike jetable avant toute architecture** pour chaque technique risquée, avec un verdict écrit (go, go avec réserves, no-go avec l'alternative) et un ADR.
- **Lancer souvent la barrière complète.** `make ci` (format, clippy en erreurs, tests, WASM, `cargo deny`, `cargo audit`, métadonnées SQL) avant chaque commit ; `make ci-full` (ajoute l'image Docker et les tests navigateur dessus) avant un jalon ou quand le Dockerfile ou une dépendance native change.
- **Répéter sur la machine réelle** dès que la phase le permet : décodage, certificats, courrier et disque ne se révèlent nulle part ailleurs.
- Aucun secret, adresse réelle ni image personnelle dans le dépôt, les journaux, les sauvegardes ou les fixtures.

## Prérequis

Rust 1.99.1 (édition 2024, épinglé dans `rust-toolchain.toml`) avec la cible `wasm32-unknown-unknown` ; `cargo-leptos` 0.3.11, `sqlx-cli`, `cargo-deny`, `cargo-audit` ; Docker avec Compose ; `ffmpeg` avec le pilote VAAPI sur la cible ; un pilote de navigateur pour les tests de bout en bout. Compiler avec un niveau de processeur de base (`x86-64-v2`), jamais `target-cpu=native`. Garder `.cargo/config.toml` tel quel : il fait compiler le serveur et le WASM de la même façon.

## Espace de travail

Un espace de travail Cargo, un binaire à rôles. Les crates par responsabilité, de l'intérieur vers l'extérieur :

| Crate | Rôle |
|---|---|
| `dominet-proto` | types de données partagés entre le serveur et le navigateur |
| `dominet-domain` | règles pures et validation, sans entrées-sorties |
| `dominet-store` | dépôts PostgreSQL et migrations |
| `dominet-mqtt` | le client MQTT supervisé |
| `dominet-automation` | moteur d'automations, planificateur, hôte de scripts, alertes, bilan d'énergie |
| `dominet-notify` | canaux e-mail et SMS |
| `dominet-integrations` | publishers météo et onduleur solaire |
| `dominet-surveillance` | caméras, ONVIF, client MediaMTX, mouvement, détection, images |
| `dominet-server` | serveur HTTP et WebSocket, services partagés par l'interface et MCP |
| `dominet-web` | l'interface Leptos |
| `dominet-mcp` | le serveur MCP |
| `dominet-agent` | la boucle de l'assistant local |
| `dominet` | le binaire : sous-commandes et rôles (`web`, `mqtt`, `scheduler`, `video`, `publishers`, `discovery`) |

## Ordre de construction

| # | Phase | Quoi construire | À lire d'abord | Fini quand |
|---|---|---|---|---|
| 0 | Spikes | Huit expériences jetables : interface en rendu serveur avec WebSocket et traductions ; stockage brut de séries temporelles à un million d'enregistrements ; live WHEP en WASM ; coût du décodage H.264 ; latence de détection d'objets ; ONVIF sur de vraies caméras ; scripts isolés ; client MQTT sous charge | Mesures §1 ; décisions sur la vidéo | chaque spike a un verdict et un ADR |
| 1 | Fondations | Configuration typée par variables d'environnement, traces, points de santé, migrations, connexion avec sessions côté serveur, mots de passe hachés et blocage, réglages typés à secrets masqués, données initiales, la coque de l'interface (mise en page, thème sombre, navigation, choix de langue, CRUD générique), pages d'administration | Pièges §1 à §4 | la connexion marche, les pages CRUD marchent, les tests d'authentification passent (blocage et courses compris), l'image se construit et démarre |
| 2 | Temps réel et tableau de bord | Hub WebSocket à sujets typés qui gardent le dernier état ; tableau de bord et widgets avec glisser-déposer et persistance ; statistiques système | Pièges §2 ; décision sur le garde-fou de rendu figé | un message met à jour un widget sans rechargement ; tests navigateur verts |
| 3 | MQTT et appareils | Écoute supervisée, ingestion Zigbee2MQTT, sources MQTT dynamiques, historique en base et dans le stockage brut, commandes d'appareils, appairage, groupes et étiquettes, publishers météo et solaire, export du contexte | Pièges §5 | une session enregistrée se rejoue en mêmes appareils, états et lignes d'historique ; coupure du courtier testée ; débit soutenu validé |
| 4 | Automations, scènes, alertes, énergie | Moteur (déclencheurs message, planning, solaire et état d'appareil ; actions publication, script, notification et scène), planificateur en processus, hôte de scripts à limites, scènes, règles d'alerte avec cycle de vie et acquittement, bilan d'énergie journalier | Pièges §5, §7 ; décisions sur les automations | des scénarios réels s'exécutent à l'identique ; les délais de relance tiennent ; un script hostile est arrêté sans nuire au processus |
| 5 | Notifications, messagerie, relais téléphone | Canaux SMTP et SMS, messagerie interne, le relais Android avec son contrat WebSocket figé | Décisions sur la messagerie | l'application téléphone non modifiée se connecte, envoie et reçoit, accuse réception ; tests de contrat verts |
| 6 | Surveillance | Caméras et synchronisation des chemins MediaMTX, live, découverte ONVIF et PTZ, backends de vision nocturne, enregistrement et relecture, snapshots, détection de mouvement, détection d'objets, l'abstraction de source d'images | Pièges §6 ; Mesures §2 | live sous 1 s sur le LAN ; une heure d'enregistrement continu sans segment perdu ; détection conforme à la référence ; processeur et mémoire mesurés sur la cible |
| 7 | Observabilité et exploitation | Point de métriques, règles et tableaux Prometheus et Grafana, sauvegardes avec restauration testée, base Compose et surcouches, proxy inverse avec TLS, Makefile, site de documentation, guide de déploiement | Pièges §8, §9 | une machine vierge s'installe en trois commandes ; une restauration est validée |
| 8 | Recette et mise en production | Checklist de parité jouée sur la machine réelle, tests de charge, répétition d'installation vierge, procédure de bascule, surveillance de 48 heures | Mesures §3, §4 ; Pièges §9 | checklist validée par le propriétaire, installation reproductible, retour arrière répété |
| 9 | Serveur MCP | Serveur intégré au binaire, jetons à portées hachés, audit écrit avant l'exécution, outils de lecture puis de pilotage puis de configuration puis d'administration, un service de configuration partagé aussi utilisé par l'interface | Pièges §11 ; décisions sur MCP | chaque outil qui modifie quelque chose a un essai à blanc ; matrice des portées testée |
| 10 | Assistant local | Boucle d'agent qui appelle les outils MCP en mémoire et un modèle auto-hébergé ; cartes d'action confirmées par l'utilisateur ; instantané, annulation et audit | Pièges §11 ; Mesures §5 | le jeu d'évaluation passe son seuil sans mauvaise carte |
| 11 | Alertes de plateforme | Vue en lecture seule des alertes Prometheus et Grafana dans la page des alertes et la pastille | Pièges §8 | les deux sources, l'état de source injoignable et la pastille sont couverts par des tests |

Dépendances : 0, 1, 2, 3, 4, 7, 8 forment le chemin critique. 5 et 6 peuvent avancer à côté de 3 et 4 une fois 2 terminée. 9 à 11 viennent après 8.

## Définition de fini de chaque phase

- Toutes les fonctionnalités sont livrées et accessibles depuis l'interface.
- Les tests unitaires, d'intégration et de contrat passent ; les chemins critiques sont couverts.
- Format, lints en erreurs, `cargo deny` et `cargo audit` sont propres.
- L'image Docker se construit et répond à sa sonde de santé.
- La documentation (deux langues) et les ADR sont à jour.
- Aucune donnée sensible dans les journaux, sauvegardes ou fixtures.
- Une démonstration pas à pas est rédigée, puis on attend la validation.

## Risques à garder en vue

| Risque | Parade |
|---|---|
| Aucun décodeur H.264 Rust pur mature | sous-processus `ffmpeg` derrière un trait de source d'images ; ne revoir qu'avec des mesures |
| Le stockage brut n'est pas une base de séries temporelles | agrégats dans PostgreSQL, historique brut dans le stockage |
| Remplacer un site d'administration généré (une trentaine d'écrans CRUD) | un composant CRUD générique dès la phase 1 |
| WebRTC et glisser-déposer en WASM | interopérabilité JavaScript minimale et isolée ; spike d'abord |
| Variété d'ONVIF selon les familles de caméras | client minimal, tests sur de vraies caméras, fixtures SOAP enregistrées |
| Images ARM | construire nativement sur une machine ARM, pas par émulation |
| Ampleur pour un développeur seul | phases courtes, parité par module, phases optionnelles laissées optionnelles |
