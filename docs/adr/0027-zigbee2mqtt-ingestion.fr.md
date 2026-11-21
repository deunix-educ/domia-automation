# 0027 — Ingestion Zigbee2MQTT : même résultat que la référence, vérifié par l'exécution de l'implémentation de référence


- Statut : accepté
- Date : 2026-11-02

## Contexte
Le critère d'acceptation de R3 est un rejeu : les mêmes messages MQTT doivent produire les mêmes appareils, états et lignes d'historique que l'application de référence. Lire le code pour le reproduire ne suffit pas (le lot 1 a déjà trouvé une
particularité de l'aplatissement des `exposes`). Il faut **exécuter** l'implémentation de référence sur les mêmes messages et comparer.

## Décisions
- **Résultat de référence figé** : le gestionnaire de messages de référence a été exécuté une fois sur un fichier de messages, sur une PostgreSQL jetable, avec une horloge virtuelle (le temps vient du fichier) pour que l'historique à la seconde soit reproductible. Le résultat (appareils et historique, en JSON) est versionné à côté du fichier de messages (`session-r3.golden.json`) : les tests Rust n'ont besoin ni de Docker ni de la référence. Les scripts qui l'ont produit ne sont pas conservés ; le fichier l'est.
- **Session de rejeu** (`tests/fixtures/z2m/session-r3.jsonl`, 64 lignes) : 11 appareils de formes variées (capteur, lumière, volet, prises, contact, présence, appareil non supporté, nom contenant un `/` ou des espaces), coordinateur, redécouverte avec métadonnées modifiées,
  appareil ajouté, états répétés dans la même seconde ou non, booléens, textes, objets imbriqués, `null`, caractères accentués, disponibilités sous toutes leurs formes, appareil inconnu, appareil désactivé puis réactivé, bruit à ignorer (`/set`, `/get`, `bridge/*`, charges non JSON ou non objet).
- **Module `dominet-automation::ingest`**, générique sur les ports de stockage et sur un `Sink` (diffusion ; plus tard automations et alertes de R4) : il ne connaît ni PostgreSQL ni le hub, donc se rejoue sur un fichier. Aiguillage (`route`) dans l'**ordre de la référence** : `bridge/devices`, `bridge/info` (objet), `…/availability`
  **avant** l'écart des sujets `bridge/…`, puis `…/set`/`…/get`, puis l'état.
- **Une tâche d'ingestion, des paquets** : les messages sont pris par paquets de 256 ; un état d'appareil est ignoré si le **message suivant concernant le même appareil** est aussi un état reçu dans la même seconde (`bridge/devices` coupe tout regroupement). Le résultat final est identique
  (état remplacé, historique à la seconde où le dernier gagne ; vérifié par le rejeu « par paquets d'une seconde »), la diffusion intermédiaire est perdue. Hors rafale, chaque paquet n'a qu'un message : aucun regroupement. Mesure : 4 000 messages (50 appareils × 80) en 1,7 s, 832 diffusions.
  **Conséquence pour R4** : sous rafale, un déclencheur « état d'appareil » ne verra que le dernier état de chaque appareil par seconde.
- **Sink serveur** (`HubSink`) : chaque changement est publié sur `device:<id>` avec **dernier état retenu** (ADR 0019) ; au démarrage, l'état connu en base de chaque appareil actif est republié, donc les widgets le montrent même après un redémarrage. `permit_join` est mémorisé dans `AppState`.
- **Préfixe Zigbee2MQTT = réglage `Z2M_BASE_TOPIC`** (comme la référence) : le modifier dans l'interface change l'abonnement (`<préfixe>/#`, QoS 1) et l'aiguillage à chaud, sans redémarrage. `upsert_discovered` reçoit ce préfixe.

## Écarts avec la référence
1. **Métadonnées dès la création (corrigé)** : la référence met `ieee`, `vendor`, `model_id`, `description` et `exposes` dans `defaults`, mais un upsert dont les valeurs par défaut ne s'appliquent qu'à la création : un appareil découvert
   reste sans métadonnées jusqu'à la republication suivante de `bridge/devices`. Ici elles sont écrites immédiatement. Le test de rejeu ne gomme pas l'écart : il exige que l'appareil concerné porte exactement les métadonnées de la dernière découverte du fichier.
2. **Noms invalides ignorés** : une découverte dont le nom contient un joker MQTT ou un caractère de contrôle est ignorée (la référence l'accepterait) ; les textes plus longs que leur colonne sont tronqués (la référence échouerait sur une erreur de base).
3. **Particularité conservée** : seuls les éléments `composite` sont aplatis pour déduire le type (ADR 0025) — l'oracle la confirme (« Salon », une lampe Zigbee2MQTT classique, reste `generic`).
4. Les messages hors du préfixe Zigbee2MQTT sont traités par les sources génériques (lot 4, ADR 0028).

## Conséquences
- Chaque évolution du rejeu (lot 4 : sources `auto_device`) étend le même mécanisme : un fichier de messages, un résultat de référence, une comparaison.
- L'oracle exige l'image locale `dominet-web` et la stack de dev ; sans eux, les tests Rust passent quand même (ils comparent au fichier versionné).
