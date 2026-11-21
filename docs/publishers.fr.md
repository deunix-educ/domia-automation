# Publishers (météo, SolarMax) et contexte

Intégrations qui lisent une source externe et **publient sur MQTT** (R3 lot 6). Décisions : [ADR 0030](adr/0030-weather-and-solarmax-publishers.md). Les messages deviennent des appareils par les [sources MQTT](mqtt.fr.md) (`meteo/#`, `solarmax/#`).

## Lancement
- `dominet serve` lance le rôle `publishers` quand `MQTT_HOST` est défini ; `--only publishers` pour un processus à part (client MQTT `…-publishers`). Les tâches sont supervisées (relancées après une panique).
- **Météo** : stations activées de `/admin/weather-stations` (Open-Meteo, sans clé). Sans station activée, aucune requête (la base est relue toutes les 30 s). Créer ou modifier une station déclenche un relevé immédiat ; ensuite, un relevé tous les `refresh_minutes` (le plus court des stations). Publié sur `meteo/<suffixe>`.
  `dominet seed-weather --lat 43.3 --lon 5.4` crée la source « Météo », l'appareil `meteo_locale` (clés suivies : température, humidité, précipitations, vent) et la station.
- **SolarMax** : seulement si `SOLARMAX_HOST` est renseigné — **il n'a aucune valeur par défaut** (jamais de contact par mégarde avec l'onduleur de la production). Un cycle toutes les `SOLARMAX_INTERVAL_SECS` s (10) : détection, grandeurs (`pac`, `eac`, `pdc`, `edc`, `qdy`, `qt0`, `tmpr`, `status`), état et alarmes ;
  une alarme de l'onduleur n'est pas publiée. Publié sur `SOLARMAX_TOPIC` (défaut `solarmax/0x6f0930a64c8b8803/prod`, le topic par défaut).

| Variable | Défaut | Rôle |
|---|---|---|
| `OPEN_METEO_URL` | `https://api.open-meteo.com/v1/forecast` | adresse de l'API (une API simulée pour les tests) |
| `SOLARMAX_HOST` | — | onduleur à interroger ; sans lui, pas de publisher SolarMax |
| `SOLARMAX_PORT`, `SOLARMAX_INVERTER_ID` | `12345`, `1` | port TCP, numéro d'onduleur (1 à 99) |
| `SOLARMAX_TOPIC` | `solarmax/0x6f0930a64c8b8803/prod` | topic de publication |
| `SOLARMAX_INTERVAL_SECS` | `10` | intervalle entre deux lectures (1 à 3600) |

## Diagnostic
- `dominet publish weather [--print-only]` : un relevé par station activée ; affiche le JSON et le publie (sauf `--print-only`).
- `dominet publish solarmax [--print-only]` : un cycle ; refuse de s'exécuter sans `SOLARMAX_HOST`.
  **L'onduleur n'accepte qu'un client TCP à la fois** : lancée pendant que le rôle `publishers` tourne (cas normal en production), la commande échoue (« n'a pas répondu à temps ») alors que le service publie normalement. Pour vérifier la lecture sans arrêter le service, regarder l'appareil SolarMax dans l'interface ou la page Journaux (`/admin/logs`, texte `solarmax`) ; pour un relevé à la main, arrêter d'abord la pile (`make prod-down`), puis la relancer (`make prod-up`).
- `dominet dump-context [--output context.yaml]` : instantané YAML des appareils actifs (type, schéma d'état déduit des valeurs courantes, clés numériques, topic de commande, emplacement), des scènes, automatisations et règles d'alerte d'appareil actives (R7 lot 8), des règles Grafana et des schémas de référence.

## Simulateurs (tests, démonstration)
`tools/sim-open-meteo.py PORT` (API météo, relevé fixe, compteur `/__count`) et `tools/sim-solarmax.py PORT` (onduleur, protocole complet) n'écoutent que `127.0.0.1`. Les tests Rust ont leur propre onduleur simulé (`dominet_integrations::solarmax::sim`). `tools/e2e.sh` les lance et pointe le serveur dessus.

## Vérification par des valeurs de référence
- `tests/fixtures/solarmax/vectors.json` contient des vecteurs de référence figés (requêtes, réponses, mises à l'échelle, arrondis), produits **sans réseau** : le constructeur, qui se connecte à l'onduleur, n'est jamais appelé. Les tests Rust exigent les mêmes résultats.
- `tests/fixtures/context/schemas.json` contient les schémas d'état de référence (`_state_schema`, `_numeric_keys`).

## Tests
`cargo test -p dominet-integrations` (protocole, client contre l'onduleur simulé, météo contre l'API simulée, supervision), `-p dominet-server --test publishers` (API et onduleur simulés → publisher → Mosquitto de dev → source → appareil), `--test context`, `-p dominet --test ui` (stations), E2E `weather`.
