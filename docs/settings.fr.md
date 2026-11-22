# Réglages applicatifs

Dix-sept réglages modifiables sans redémarrage, stockés en base (`app_setting`), typés (`str`, `int`, `float`, `bool`) et
groupés par catégorie. Un réglage absent de la base prend sa **valeur par défaut** : l'application fonctionne avant `dominet seed`.

| Catégorie | Clé | Type | Défaut |
|---|---|---|---|
| Notifications | `EMAIL_RECIPIENT` | str | `dominet@localhost` |
| Notifications | `DEFAULT_FROM_EMAIL` | str | `dominet@localhost` |
| Notifications | `SMS_FREE_USER` | str | *(vide)* |
| Notifications | `SMS_FREE_KEY` | str, **secret** | *(vide)* |
| Alertes | `PROMETHEUS_ALERT_COOLDOWN` | int (s) | `3600` |
| Alertes | `MOTION_ALERT_COOLDOWN` | int (s) | `600` |
| Surveillance | `MOTION_THRESHOLD` | float (0.0–1.0) | `0.01` |
| Surveillance | `RECORDINGS_SEGMENT_DURATION` | int (s) | `60` |
| Interface | `GRAFANA_PUBLIC_URL` | str | `http://localhost:3000` — origine autorisée pour les iframes Grafana du tableau de bord (CSP `frame-src`, ADR 0022) |
| Interface | `PROMETHEUS_PUBLIC_URL` | str | `http://localhost:9090` |
| Interface | `MEDIAMTX_WEBRTC_DOMAIN` | str | `localhost:8889` |
| Interface | `MEDIAMTX_WEBRTC_SCHEME` | str | `http` |
| MQTT | `Z2M_BASE_TOPIC` | str | `zigbee2mqtt` |
| Assistant | `AGENT_ENABLED` | bool | `false` — active la page Assistant (R12) |
| Assistant | `AGENT_LLM_URL` | str | *(vide)* — adresse de `llama-server`, ex. `http://192.168.1.20:8081` |
| Assistant | `AGENT_MODEL` | str | `Qwen3-8B-Q4_K_M` — demandes courantes |
| Assistant | `AGENT_MODEL_ADMIN` | str | *(vide)* — modèle des demandes de configuration ; vide = le même que `AGENT_MODEL` |

Valeurs : décimaux avec un **point** (`0.01`), booléens `true/false/oui/non/1/0`. Une valeur incompatible avec le type est refusée (400).

## Secrets

Une clé contenant `KEY`, `PASSWORD`, `TOKEN` ou `SECRET` est un **secret** : chiffrée en base (AES-256-GCM, ADR 0011),
jamais renvoyée par l'API (`********`), jamais journalisée. Elle exige `DOMINET_MASTER_KEY` :

```bash
dominet gen-master-key          # affiche une clé : à copier dans DOMINET_MASTER_KEY du fichier .env
```

**Sauvegardez cette clé à part de la base** : sans elle, les secrets enregistrés sont perdus.

## Données initiales

```bash
dominet seed [--dry-run]
```

- Crée les réglages absents. Valeur initiale : variable d'environnement du même nom si elle existe (utile en Docker, ex.
  `SMS_FREE_KEY` dans `.env`), sinon valeur par défaut. **N'écrase jamais** un réglage existant ; relançable sans risque.
- Crée la navigation par défaut (7 entrées) **uniquement si la table est vide** : une entrée supprimée ou renommée par
  l'administrateur n'est pas recréée.
- Vérifie tout avant d'écrire : en cas d'erreur (secret sans clé maîtresse, valeur invalide), rien n'est écrit.
- `--dry-run` montre ce qui serait créé sans rien écrire. Les valeurs ne sont jamais affichées.

## Routes (administrateur)

| Route | Rôle |
|---|---|
| `GET /api/settings` | les 17 réglages (valeur affichable, `is_secret`, `is_set`, `from_default`, `readable`) |
| `PUT /api/settings/{key}` `{"value": "..."}` | modifie un réglage ; 400 si invalide, 404 si inconnu, 503 si secret sans clé maîtresse |

Chaque modification met le cache à jour immédiatement et prévient les composants abonnés (reconfiguration à chaud).
