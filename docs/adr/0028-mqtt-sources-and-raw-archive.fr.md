# 0028 — Sources MQTT, historique et archive brute ReductStore


- Statut : accepté
- Date : 2026-11-02

## Contexte
Le lot 3 écoute `zigbee2mqtt/#`. L'implémentation de référence écoute en plus chaque `MqttSource` active (Linky, SolarMax, météo…), peut créer un appareil à chaque message (`auto_device`), historise les clés suivies (PostgreSQL) et — décision R0, ADR 0002 —
l'IoT brut doit aussi aller dans ReductStore. Le lot 4 ajoute ces trois choses et la page d'administration des sources.

## Décisions
- **Sources dynamiques.** L'ingestion garde la liste des sources actives (`Ingestor::set_sources`) ; un message hors du préfixe Zigbee2MQTT est confronté au filtre de chaque source (`topic_matches`, MQTT 3.1.1). La modification d'une source dans l'administration
  prévient l'écoute (`AppState.sources_changed`, un `Notify` en processus) qui relit les sources, met à jour l'aiguillage **et** les abonnements (différence seulement : `set_subscriptions`). Un rechargement de sécurité toutes les 60 s couvre le cas où l'administration et
  l'écoute tournent dans deux processus (`--only web`, `--only mqtt`). Un même filtre déclaré deux fois (ou égal à `<préfixe>/#`) est souscrit une fois, au QoS le plus élevé.
- **Comportement repris de la référence** : pour chaque source qui correspond — journal si `log_messages` (topic et début de la charge utile, 512 caractères), appareil automatique si `auto_device` **et** charge utile objet JSON ; ensuite, si au moins une
  source correspondait, les appareils actifs dont `base_topic/nom` ou `base_topic` égale le topic sont marqués vus. L'appareil automatique prend pour nom le topic sans le préfixe de la source (sinon le topic entier), remplace son état, est **réactivé** s'il était désactivé,
  est diffusé en temps réel et historisé comme un appareil Zigbee2MQTT.
- **`handler_code`** est conservé, modifiable et affiché dans l'administration, jamais exécuté avant R4 (Rhai, ADR 0006).
- **Archive brute ReductStore (`iot`)**, facultative (`REDUCTSTORE_URL`). Un enregistrement par message qui écrit au moins une ligne d'historique : corps = état JSON complet, une étiquette par clé suivie numérique (les booléens valent 0/1), entrée `dev_<id>`, horodatage en µs.
  Le nom du bucket est `iot` (réglable : `REDUCTSTORE_IOT_BUCKET`, pour les tests), quota FIFO `IOT_QUOTA_GB` (défaut 5) appliqué **à la création** du bucket. L'écriture passe par une **file bornée (10 000)** et une tâche qui écrit par lots de 50 (limite des en-têtes HTTP relevée
  par le spike) : l'ingestion n'attend jamais ReductStore. Serveur injoignable : le bucket est recréé à la tentative suivante, les enregistrements qui ne passent pas sont **abandonnés et comptés** (un avertissement à la première coupure, un message à la reprise). Deux messages d'un même appareil dans la même
  microseconde reçoivent des horodatages décalés (ReductStore refuse les doublons).
- **PostgreSQL reste la source des graphiques.** `device_state_log` est purgé une fois par jour au-delà de `HISTORY_RETENTION_DAYS` (défaut **400**, D7 du plan R3 ; l'ADR 0002 proposait 30 jours à titre provisoire). Variables d'environnement plutôt que réglages applicatifs : comme
  `REDUCTSTORE_URL`, elles décrivent le déploiement, pas le comportement de la maison, et l'ajout de clés au registre des 13 réglages de la référence n'apporte rien ici.
- **Page `/admin/mqtt-sources`** (administrateurs) : le composant CRUD générique (ADR 0014), fonctions serveur typées `list/save/delete`, validation champ par champ par le domaine (`MqttSource::problems` : filtre MQTT valide, QoS 0 à 2, longueurs), préfixe nettoyé de son `/` final.

## Écarts avec la référence
1. **État dès la création de l'appareil automatique** : la référence écrit `state`, `available` et `last_seen` dans `defaults`, avec un upsert dont les valeurs par défaut ne s'appliquent qu'à la création : le premier message d'une source ne laisse **aucun état** en base
   (il n'est que diffusé). Ici l'état est écrit tout de suite (même défaut que celui corrigé à l'ADR 0027).
2. **Noms invalides ignorés** : un topic qui ferait un nom d'appareil invalide (plus de 128 caractères, joker, caractère de contrôle) est ignoré, au lieu d'une erreur de base.
3. **Pas de « réessaie une fois »** sur perte de connexion à la base : le pool de connexions se reconnecte, un message perdu est repris par le QoS 1 du broker tant que la session persiste.

## Conséquences
- Sous coupure de ReductStore, l'historique PostgreSQL continue ; seule l'archive brute a un trou (compté dans le journal, pas encore dans une métrique : R7).
- Changer `IOT_QUOTA_GB` n'agit pas sur un bucket existant (à régler dans ReductStore ou en supprimant le bucket) : limite documentée dans `docs/mqtt.md`.
- Les publishers du lot 6 (météo, SolarMax) publient sur le broker : leurs topics se déclarent comme n'importe quelle source (`meteo/#`, `solarmax/#`) ; rien de spécial côté ingestion.
- Le client HTTP de ReductStore (rustls) ne se construit pas sans certificats racine, et `reduct-rs` paniquait alors : l'image Docker installe `ca-certificates`, et le serveur utilise `try_build` — en cas d'échec il démarre **sans archive** (avertissement) au lieu de s'arrêter. Découvert par l'E2E sur l'image (`make ci-full`), pas par les tests sur l'hôte.
