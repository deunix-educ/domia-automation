# 0020 — Statistiques système : `sysinfo`, publiées comme dernier état du sujet `system`


- Statut : accepté
- Date : 2026-11-01

## Contexte
L'implémentation de référence mesure CPU, mémoire et disque toutes les 5 s (`psutil`, thread de fond) et chaque onglet interroge `/api/system-stats/` toutes les 5 s. R2
reprend les jauges de la barre latérale, y ajoute la température (ROADMAP) et dispose désormais d'un hub WebSocket avec dernier état conservé (ADR 0019).

## Options
**Source des mesures.** (a) Lire `/proc` et `/sys` à la main — peu de code mais fragile (formats, conteneurs, autres plateformes) ; (b) `procfs` / `sysinfo` ;
(c) **`sysinfo`** (MIT, sans `unsafe` chez nous, API unifiée processeur / mémoire / disques / capteurs) — retenu.

**Diffusion.** (a) Les onglets interrogent l'API en boucle, comme dans la référence — N requêtes authentifiées (lecture de session en base) toutes les 5 s par onglet ;
(b) **le collecteur publie sur le sujet `system`** (dernier état conservé) et les onglets s'abonnent — un seul calcul, une seule publication, nouvel onglet servi
au premier battement — retenu. L'API REST subsiste (compatibilité avec la référence, clients sans WebSocket, scripts) et **relit le même dernier état** : pas de second cache,
pas de mesure à la demande, donc aucune charge ajoutée par un client indiscret.

## Décision
- `dominet-server::system_stats` : `Sampler` (instruments `sysinfo` conservés d'un relevé à l'autre, indispensable à la charge CPU qui est une différence), lecture
  **hors du réacteur** (`spawn_blocking`), première mesure après `MINIMUM_CPU_UPDATE_INTERVAL`, puis toutes les `SYSTEM_STATS_SECS` secondes (défaut 5, 1 à 3600).
- Conversion en `SystemStatsDto` par des fonctions **pures** (`build`, `pick_temperature`), testées sans machine : Gio et pourcentages à une décimale (comme
  la référence), pourcentages bornés à 0–100 et jamais `NaN`, mémoire utilisée = total − disponible, disque = `/` uniquement.
- Format : les clés de la référence (`cpu{percent,count}`, `memory{percent,used_gb,total_gb,available_gb}`, `disk{percent,used_gb,total_gb,free_gb}`, `updated_at` en
  secondes Unix) **plus `temperature`**. `disk` et `temperature` valent `null` quand la machine ne les expose pas : une valeur absente plutôt que fausse
  (la référence renvoyait 0). Sans aucune mesure (démarrage, collecteur arrêté), l'API répond **503**, pas des zéros.
- Température : le capteur processeur d'abord (`coretemp`, `package`, `cpu`, `tctl`, `k10temp`, `soc`), sinon le plus chaud ; valeurs hors `0 < t ≤ 150` ignorées.
- `GET /api/system-stats/` exige un compte connecté (401 sinon ; la référence redirigeait vers la page de connexion), tous rôles.
- Navigateur : composant `SystemGauges` abonné au sujet `system` ; les barres sont des `div` dont la **largeur est posée par la CSSOM** (`style:width`), seule voie
  compatible avec la CSP sans `unsafe-inline` ; valeurs « … » tant qu'aucune mesure n'est reçue ou si la connexion est coupée (pas de chiffre périmé).

## Conséquences
- Dans Docker, `/proc` décrit l'hôte (CPU, mémoire) mais le disque est celui du conteneur et la température exige `/sys/class/thermal` monté (R7) ; sinon `null`.
- Un sujet `system` toujours actif coûte un message par intervalle même sans abonné (négligeable : quelques centaines d'octets) ; arrêtable plus tard en
  suspendant le collecteur sans abonné si besoin.
- `sysinfo` ajoute des dépendances transitives (`libc`, `windows` non compilé sous Linux) : `cargo deny` et `cargo audit` les contrôlent.
