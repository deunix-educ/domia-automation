# domia-automation

**English : [README.md](README.md)**

Application de domotique et de vidéosurveillance auto-hébergée, écrite en Rust, avec un **serveur MCP** et un **assistant IA local** facultatif. Elle tourne sur une petite machine x86-64 (un mini-PC Intel Core i5 tient 12 caméras en 1080p, voir [Mesures](docs/measures.fr.md)) et s'installe avec Docker Compose.

**Origine.** DomiNet est née comme l'application domotique personnelle de son auteur, construite avec Django, Channels et Redis. Ce dépôt en est la réécriture en Rust (Axum, Leptos, PostgreSQL) ; le nom du produit dans le code reste `dominet`.

**Statut.** Publiée : la version actuelle est la 1.1.0 (voir le [journal des modifications](CHANGELOG.md)). La partie 1 (automations, vidéosurveillance, observabilité, serveur MCP) et la partie 2 (assistant conversationnel local) sont construites ; la partie 3 (assistant vocal) est prévue.

## Ce qu'elle fait

- **Serveur MCP** : un assistant (Claude Code, Claude Desktop, tout client MCP) lit, pilote et configure l'application selon les mêmes règles que l'interface, avec des jetons à portées, un journal d'audit et sans jamais voir un secret. [Serveur MCP](docs/mcp.fr.md)
- **Assistant IA local** : posez vos questions dans l'interface (« Quels volets sont encore ouverts ? », « Ferme-les »). Un modèle que vous hébergez (llama.cpp) propose des actions que vous confirmez d'un clic ; chaque changement est journalisé et peut être annulé. Rien ne sort de votre réseau. Il répond en français. [Assistant](docs/assistant.fr.md)
- **Appareils et automations** : Zigbee2MQTT et sources MQTT génériques, scènes, plannings, lever et coucher du soleil, scripts isolés, règles d'alerte avec délai de relance et acquittement.
- **Vidéosurveillance** : découverte ONVIF et PTZ, live dans le navigateur, enregistrement continu ou sur mouvement, détection de mouvement et d'objets sur votre propre matériel. Chaque spectateur a ses propres identifiants pour des lecteurs comme VLC ([Accès aux caméras](docs/camera-access.fr.md)).
- **Énergie** : bilan journalier et tableau Grafana avec le taux d'autoconsommation.
- **Notifications et Android** : e-mail, SMS et relais Android : l'application **DomoComm** envoie au serveur les SMS et les appels d'un téléphone, et envoie des SMS sur demande. APK à télécharger sur les [releases](https://github.com/deunix-educ/domia-automation/releases) (étiquettes `android-*`), voir [Relais Android](docs/android.fr.md).
- **Accès à distance** : un VPN WireGuard facultatif (wg-easy 15) pour utiliser l'application hors de chez soi.
- **Observabilité** : Prometheus et Grafana, avec les alertes de plateforme affichées dans l'application.
- **Déménager** : export et import d'une installation complète, pour changer de machine ([Migration](docs/migration.fr.md)).

## Installer

Débutant ? Clonez le dépôt et lancez `./install.sh` : il vérifie la machine, pose quatre questions et fait le reste ([installation rapide](docs/quick-install.fr.md) ; `./install.sh --dry-run` ne change rien). Installation manuelle en production en trois commandes : [`DEPLOY.md`](DEPLOY.md). La procédure complète, pas à pas, est dans [`docs/installation.md`](docs/installation.fr.md).

## Documentation

La documentation se construit avec `make docs` et est aussi servie par l'application sur `/docs/`. Pour commencer :

- [Pièges et leçons](docs/pitfalls.fr.md) : tous les pièges rencontrés à la construction, par domaine
- [Mesures](docs/measures.fr.md) : ce que tient le matériel, avec la façon de refaire chaque mesure
- [Décisions par thème](docs/decisions.fr.md) et les [ADR](docs/adr/) : pourquoi c'est construit ainsi, y compris ce qui a été abandonné
- [Reconstruire de zéro](docs/rebuild.fr.md) : l'ordre de construction

## Développement

```bash
make help   # cibles disponibles
make ci     # format, clippy, tests, WASM, deny, audit
make hooks  # active la barrière pre-push (make ci sur une copie propre)
make e2e    # tests navigateur de bout en bout (voir docs/tests.md) ; make ci-full ajoute l'image Docker
make docs   # documentation MkDocs dans target/docs-site
```

Prérequis : `rustup` (la chaîne d'outils est épinglée dans `rust-toolchain.toml`) et Docker. Détails dans [`docs/dev-machine.md`](docs/dev-machine.fr.md).

Il n'y a pas de CI automatique : la barrière est `make ci`, lancé en local et par le hook `pre-push`.

## Licence

AGPL-3.0-or-later. English : [README.md](README.md). Composants tiers et leurs licences, avec le degré de vérification de chacun : [Licences](docs/licenses.fr.md). Données météo par [Open-Meteo.com](https://open-meteo.com/) (CC BY 4.0 ; l'API gratuite est réservée à un usage non commercial).
