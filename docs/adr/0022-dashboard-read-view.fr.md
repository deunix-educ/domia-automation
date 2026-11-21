# 0022 — Tableau de bord en lecture : positions par classes, HTML assaini, iframes Grafana limitées


- Statut : accepté
- Date : 2026-11-02

## Contexte
Le lot 4 de R2 remplace la page d'accueil provisoire par le tableau de bord (lecture seule ; l'édition vient au lot 5). Trois contraintes de sécurité et de
robustesse orientent la conception : la CSP interdit l'attribut `style` (l'implémentation de référence place chaque widget par `style="grid-column: …"`), le contenu du widget `text` est du
HTML saisi par un administrateur (affiché tel quel par la référence), et Grafana s'affiche dans une iframe que la CSP bloque par défaut.

## Décision
- **Placement par classes** : un widget porte `vc-c-N` (colonne N de 1 à 12), `vc-r-N` (ligne 1 à 100), `vc-w-N` et `vc-h-N` (largeur et hauteur, 1 à 12). Ces classes posent des
  variables CSS (`--c`, `--r`, `--w`, `--h`, plus `--w8` et `--w4`, la largeur ramenée à 8 puis 4 colonnes) et une seule règle `.vc-widget` en déduit `grid-column` / `grid-row`.
  Écrans moyens (≤ 1200 px) et étroits (≤ 768 px, ≤ 480 px) : les widgets se placent à la suite (`auto / span …`) au lieu de garder leur colonne d'origine, qui sortirait de la
  grille réduite. **La ligne maximale du domaine passe de 999 à 99** (le CSS couvre 100 lignes de 120 px, 12 000 px : largement assez) ; la contrainte SQL de la migration 0007
  reste à 999 (le domaine est plus strict, ce qui ne demande aucune migration).
- **HTML du widget `text` assaini à l'affichage** (`dominet_server::html::sanitize_widget_html`, crate `ammonia`) : mise en forme, listes, tableaux, liens et images ; ni script, ni iframe,
  ni formulaire, ni `style`, ni gestionnaire d'événement ; liens `http`, `https`, `mailto`, `tel` seulement, avec `rel="noopener noreferrer"` ; les attributs `id` et `class` sont retirés.
  Le serveur renvoie du HTML déjà sûr (`content_html`) : le navigateur n'a aucune décision de sécurité à prendre. Le même assainissement servira à l'enregistrement (lot 5) :
  la base n'est pas une source fiable (SQL à la main, import). Le repli de référence « texte brut de la configuration » devient un texte échappé avec retours à la ligne.
- **Iframes Grafana limitées à une origine** : l'adresse est calculée côté serveur (`widget_view::grafana_src`, mêmes cas que la référence : `panel_url`, appareil + `key`, `dashboard_uid` +
  `panel_id`) et **n'est chargée que si son origine est celle du réglage `GRAFANA_PUBLIC_URL`** ; sinon le widget affiche « adresse refusée ». La CSP gagne `frame-src <origine>`
  (lue dans le cache des réglages, `SettingsService::cached_string`, qui est chargé au démarrage pour que l'en-tête n'ait pas à interroger la base). Les `uid` et `key` sont
  filtrés (caractères alphanumériques, `-`, `_`) : ils ne peuvent pas injecter de paramètres. L'iframe porte `sandbox="allow-scripts allow-same-origin"`.
- **Visibilité** : un compte ne voit que **ses** tableaux (comme la référence). `?d=<id>` d'un autre compte, absurde ou inexistant retombe sur le tableau par défaut, sans rien révéler (testé).
  L'administration de tous les tableaux viendra avec le lot 5.
- **Appareils sans table (R3)** : le rendu d'un widget `device` part du **dernier état retenu** du hub (rendu serveur), puis suit `device:<id>` en direct (abonnement compté, ADR 0019).
  La présentation automatique est déduite des clés de l'état (`widget_view::resolve_presentation` : lumière, volet, climatisation, ventilateur, serrure, météo, commutateur, capteur binaire,
  capteur, générique) ; les unités viennent d'une table de clés Zigbee2MQTT. Les commandes (ON/OFF, curseurs, +/−…) sont **présentes mais désactivées** jusqu'à R3.
  Caméras et scènes sont des emplacements annoncés (R6, R5).
- **Dépendances** : `dominet-domain` devient utilisable dans le WASM (`uuid` sans la fonctionnalité `v4`, jamais utilisée) ; `ammonia` côté serveur.

## Conséquences
- Une ligne au-delà de 99 est refusée par le domaine ; un widget plus large que l'écran se place à la suite plutôt qu'à sa colonne d'origine.
- La CSP contient désormais `frame-src` sur toutes les pages (y compris la connexion) : surface minime (une origine, configurable), mais non nulle ; elle disparaît si le réglage est vide ou invalide.
- Un widget Grafana dont l'adresse change d'origine après coup (réglage modifié) affiche « adresse refusée » au lieu de charger une iframe bloquée par le navigateur.
- Le sélecteur de tableaux est un `<details>` natif : aucun script, accessible au clavier.
