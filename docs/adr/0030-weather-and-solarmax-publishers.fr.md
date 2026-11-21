# 0030 — Publishers météo et SolarMax, supervisés dans le serveur


- Statut : accepté
- Date : 2026-11-02

## Contexte
L'implémentation de référence lance deux processus séparés (`weather_publisher`, `solarmax_publisher`, classe `DomiNetPublisher`) qui lisent une source externe et publient sur MQTT ; le pipeline des sources MQTT (lot 4) en fait des appareils. Le SolarMax y est contacté à une adresse **codée en dur**
(`192.168.1.123`), celle de l'onduleur de la production. Règle du projet : aucun contact avec l'onduleur avant R8, sans accord explicite.

## Décisions
- **Tâches du serveur** (D5 du plan R3) : un rôle `publishers` (nouveau, actif par défaut quand `MQTT_HOST` est défini) lance des tâches **supervisées** (`supervise` : une panique relance la tâche avec une pause de 1 à 60 s). Il publie par le client du rôle `mqtt` s'il tourne, sinon par un client à lui
  (identifiant `…-publishers`, aucun abonnement). `--only publishers` permet un processus à part, comme dans la référence. Les boucles vivent dans `dominet-integrations` et ne connaissent que le trait `Publisher` : elles se testent sans broker.
- **Météo** : la tâche tourne toujours mais n'interroge l'API que pour les stations **activées** (aucune station, aucune requête). Un cycle parcourt les stations activées ; une station en échec n'arrête pas les autres. La pause est le plus court `refresh_minutes` des stations activées
  (30 s sans station : une relecture de la base, sans requête à l'API, pour voir vite une station créée par `dominet seed-weather` dans un autre processus ; la référence attendait 10 min) et la tâche est **réveillée** quand une station est créée ou modifiée dans l'administration (`AppState.stations_changed`) : pas d'attente jusqu'au prochain cycle. Même requête et même normalisation que la référence (dix valeurs, `null` si absente). L'adresse de l'API est
  `OPEN_METEO_URL` (défaut : l'API publique) : tests et CI parlent à une API **simulée**.
- **SolarMax** : le protocole est porté en Rust (`solarmax::protocol`, pur) et vérifié **contre l'exécution de l'implémentation de référence**. Requêtes identiques octet pour octet, mêmes mises à l'échelle, même arrondi (demi-pair : `round(115.25, 1)` vaut 115.2 dans la référence et ici). Le client TCP asynchrone est borné dans le temps, abandonne la connexion à la moindre erreur et la reprend au cycle suivant. Une alarme de l'onduleur
  (`SAL` ≠ 0) est une erreur : rien n'est publié, comme dans la référence. Le topic de publication est un réglage (`SOLARMAX_TOPIC`, défaut celui de la référence).
- **Aucune adresse d'onduleur par défaut.** Sans `SOLARMAX_HOST`, aucun publisher SolarMax et `dominet publish solarmax` refuse de s'exécuter. Les tests utilisent un **onduleur simulé** (serveur TCP sur `127.0.0.1`, Rust pour les tests unitaires, la référence pour les E2E) ; le même simulateur la référence répond
  octet pour octet comme l'implémentation de référence de l'oracle.
- **Diagnostic** : `dominet publish weather|solarmax [--print-only]` fait **un** cycle, affiche le résultat et publie (sauf `--print-only`). Le plan parlait de `--once` : un cycle unique est le seul comportement, le drapeau serait sans objet.
- **Administration** : `/admin/weather-stations` (CRUD générique, virgule décimale acceptée, tous les problèmes du formulaire rapportés ensemble). `dominet seed-weather [--lat --lon …]` crée la source « Météo » (`meteo/#`, appareils automatiques), l'appareil `meteo_locale` et la station.
- **`dominet dump-context`** : instantané YAML des appareils actifs (type, schéma d'état déduit des valeurs courantes, clés numériques, topic de commande, emplacement) et schémas de référence ; scènes, automatisations et alertes sont des listes vides jusqu'à R4. Le schéma d'état est vérifié contre l'implémentation de référence
 . Le YAML est écrit par un émetteur de quelques dizaines de lignes (aucune bibliothèque YAML maintenue n'est nécessaire) : chaînes toujours entre guillemets, clés réservées entre guillemets.
- `serde_json` passe en `preserve_order` : l'ordre des clés d'un état est celui de la base (comme un dictionnaire ordonné), donc le même que celui de l'implémentation de référence dans `dump_context`.

## Écarts avec la référence
1. Pas de faux état initial pour l'appareil météo (`seed_meteo` y écrivait 13 °C…) : il attend le premier relevé réel. Le widget de tableau de bord créé par `seed_meteo` n'est pas repris (le widget se choisit dans l'administration).
2. Le fabricant et le modèle de l'appareil météo (`Open-Meteo`, `API`) ne sont pas renseignés : un appareil créé à la main ne porte pas ces champs de découverte.
3. Le topic météo est `meteo/<suffixe>` (la propriété `mqtt_topic` de la référence affichait `zigbee2mqtt/<suffixe>`, jamais utilisée pour publier).
4. Une publication ne crée plus une connexion MQTT par message (la référence se connectait, publiait et se déconnectait à chaque cycle) : le client du serveur reste connecté.

## Conséquences
- La coupure du broker interrompt les publications (refusées tout de suite, journalisées) ; elles reprennent au cycle suivant sans intervention.
- Un second processus `--only publishers` et le rôle `mqtt` d'un autre ne doivent pas tourner en même temps avec le même identifiant de client (le suffixe `-publishers` l'évite).
- Le contact réel avec l'onduleur et avec l'API Open-Meteo reste réservé à R8 / à la démonstration avec accord.
