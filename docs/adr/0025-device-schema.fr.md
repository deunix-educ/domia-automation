# 0025 — Schéma des appareils : un état remplacé, un historique à la seconde, des images en base


- Statut : accepté
- Date : 2026-11-02

## Contexte
R3 introduit les appareils (Zigbee2MQTT, sources MQTT génériques), leur historique, des images, les sources MQTT et les stations météo. Le critère d'acceptation est un **rejeu** : les mêmes messages MQTT doivent
donner les mêmes `Device`, états et lignes d'historique que l'implémentation de référence. Le schéma et les règles du domaine reprennent donc la sémantique de l'implémentation de référence, y compris ses particularités, tout en profitant de PostgreSQL.

## Décisions
- **Tables** (migration 0008) : `device_group`, `device_label`, `device`, `device_image`, `device_state_log`, `mqtt_source`, `weather_station`. Clés primaires `BIGINT` identité ; `friendly_name` unique et sensible à la casse comme dans la référence.
- **État remplacé, pas fusionné** : `device.state` (objet JSON) est **remplacé** à chaque message, `last_seen = now()`, `available = true` ; la disponibilité (`…/availability`) ne touche pas `last_seen`. Les opérations de l'écoute
  MQTT sont des méthodes du dépôt (`apply_state`, `set_available`, `upsert_discovered`, `upsert_auto`, `touch_by_topic`) : une instruction SQL chacune, donc **idempotentes** (les doublons QoS 1 de l'ADR 0005 n'ont d'autre effet que `last_seen`).
- **Listes de clés en `TEXT[]`** (`tracked_keys`, `hidden_keys`) plutôt qu'en JSON : le type garantit la forme (liste de textes) ; `exposes` et `state` restent en `JSONB`. Écart avec la ROADMAP §3.1, qui laissait le choix.
- **Historique** : `device_state_log(device_id, key, ts, value_num, value_str)` à la seconde, `UNIQUE(device_id, key, ts)`, écriture par lot (`UNNEST`) avec **remplacement** d'une ligne existante du même instant. Index `(device_id, key, ts DESC)` pour les graphiques et
  `(ts)` pour la purge (rétention configurable, lot 4). Mêmes colonnes que dans la référence : les tableaux Grafana n'auront qu'un nom de table à changer (R7).
- **Découverte** (`bridge/devices`) : création avec le préfixe `zigbee2mqtt`, la clé masquée `update` et un type déduit des `exposes` ; un appareil existant **garde** son type, ses clés suivies, son icône et son état (seules les métadonnées découvertes sont rafraîchies).
  **Particularité de la référence conservée** : seuls les éléments `composite` des `exposes` sont aplatis pour déduire le type, donc un `light`, un `switch` ou un `cover` Zigbee2MQTT (qui portent des `features` mais ne sont pas `composite`) restent `generic`. Le rejeu contre l'implémentation de référence vérifie
  cette équivalence ; corriger la déduction est une amélioration à décider à part. Les unités, elles, aplatissent toutes les `features`.
- **Sources `auto_device`** : l'appareil est créé ou mis à jour **et réactivé** (`active = true`), sans que son type ne change.
- **Images** : table `device_image` (JPEG réduit, 5 Mio au plus, types vérifiés) servie par une route à `ETag` (lot 5) : pas de volume à sauvegarder à part. `image_url` (URL externe) reste possible.
- **Noms et topics** : un nom d'appareil sert à construire des topics ; le domaine refuse caractères de contrôle, jokers MQTT (`+`, `#`), `/` de tête ou de queue et `//` (`is_valid_friendly_name`). Une découverte au nom invalide est ignorée plutôt que d'échouer.
  Un filtre d'abonnement (`mqtt_source.topic_pattern`) doit être un filtre MQTT valide (`+` sur un niveau entier, `#` dernier niveau).
- **Clé étrangère `widget.device_id`** (annoncée par l'ADR 0018) : `ON DELETE SET NULL` ; les références orphelines sont mises à `NULL` avant d'ajouter la contrainte. Une référence à un appareil inexistant est refusée à l'enregistrement du widget (champ `device_id`).
- **`handler_code`** des sources : conservé tel quel (≤ 20 000 caractères) et affiché, **exécuté à partir de R4** (Rhai) ; le code écrit dans une autre syntaxe n'est pas portable.
- **Libellés** : `KNOWN_LABELS` (25 clés, repris de `seed_device_labels`) est dans le domaine ; `dominet seed-labels [--dry-run]` crée les libellés manquants des clés présentes sur les appareils, sans jamais écraser.

## Conséquences
- 21 tables ; les scripts E2E de R2 qui utilisaient des numéros d'appareil fictifs créent désormais leurs appareils (le simulateur d'état n'est restreint aux appareils existants qu'au lot 5).
- Le rejeu contre le résultat de référence (lot 3) tranchera tout écart de sémantique ; la déduction du type de `light`/`switch`/`cover` est la première particularité à surveiller.
- Aucune migration de données de production ici : la reprise des appareils et de l'historique relève de R7/R8.
