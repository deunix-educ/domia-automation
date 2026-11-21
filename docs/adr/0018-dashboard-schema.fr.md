# 0018 — Schéma du tableau de bord : exclusion de chevauchement et références sans clé étrangère


- Statut : accepté
- Date : 2026-11-01

## Contexte
R2 introduit `dashboard` et `widget`. Deux difficultés : (1) deux widgets d'un même tableau ne doivent jamais se chevaucher, y compris quand deux
déplacements arrivent en même temps ; (2) un widget désigne un appareil (R3), une scène (R5) ou une caméra (R6), tables qui n'existent pas encore.

## Options
**Chevauchement.** (a) Vérifier en Rust avant d'écrire — non atomique : deux requêtes simultanées passent toutes les deux. (b) Verrou applicatif
seul — correct mais n'empêche pas un autre code (CLI, migration, SQL à la main) de casser l'invariant. (c) **Contrainte d'exclusion PostgreSQL**
(`EXCLUDE USING gist`, extension `btree_gist`) — l'invariant vit dans la base — retenue.

**Références.** (a) Créer des tables `device`/`scene`/`camera` minimales maintenant — empiète sur R3/R5/R6 et fige un schéma qu'on ne connaît pas encore.
(b) **Colonnes sans clé étrangère** (`device_id`, `scene_id` entiers, `camera_uuid` UUID), la clé étant ajoutée par la migration qui crée la table visée —
retenue. (c) Stocker les références dans le JSON `config` — perd l'intégrité et les requêtes.

## Décision
- Grille de 12 colonnes ; `grid_col`, `grid_row`, `width`, `height` (les noms `col`/`row` de l'implémentation de référence sont des mots délicats en SQL), toutes bornées par des
  `CHECK` (`grid_col + width <= 12`). Le rectangle d'un widget est un `box(point(col, row), point(col + width - 1, row + height - 1))` en coordonnées de
  cellules, bornes incluses : des widgets contigus passent, un chevauchement est refusé (`widget_no_overlap`, SQLSTATE `23P01`, rapporté par le dépôt comme
  `RepoError::Conflict`). Testé sur la base (contraintes) et sur les dépôts (courses).
- **Écritures de widgets sérialisées par tableau** : le dépôt prend un verrou de ligne sur le tableau avant d'insérer, modifier ou déplacer. Sans cela,
  deux insertions simultanées sur les mêmes cellules se bloquent mutuellement dans la vérification de la contrainte (`deadlock detected`, comportement
  documenté de PostgreSQL) au lieu d'être refusées proprement ; un test de course (6 insertions concurrentes, 10 tours) l'a mis en évidence.
- **Tableau par défaut** : index unique partiel (`WHERE is_default`) = au plus un par propriétaire. Le dépôt prend un verrou sur la ligne du propriétaire,
  relit l'état *après* le verrou (une lecture avant donnait une valeur périmée : la course l'a révélé), et fait passer le statut d'un tableau à l'autre dans la
  même transaction. Le premier tableau d'un compte est son défaut ; le statut ne se retire pas directement, il passe à un autre tableau.
- Suppression en cascade : utilisateur → tableaux → widgets.
- `presentation` (texte, `''` = automatique), `config` (objet JSON de 4 Ko au plus) et `content` (HTML du widget `text`, 20 000 caractères au plus) sont bornés en base
  **et** validés par le domaine (`WidgetDraft::problems`, par champ, comme pour les emplacements) : cohérence type / présentation / références, configuration Grafana
  (`panel_url` http(s), ou `dashboard_uid` + `panel_id`, ou `key` avec un appareil).

## Conséquences
- La base de production doit pouvoir exécuter `CREATE EXTENSION btree_gist` (extension « de confiance » depuis PostgreSQL 13, incluse dans l'image `postgres:16`).
- Les migrations de R3, R5 et R6 devront ajouter les clés étrangères `widget.device_id`, `widget.scene_id`, `widget.camera_uuid` (`ON DELETE SET NULL` comme dans la référence) et
  traiter les références orphelines éventuelles.
- Un widget dont la cible a disparu garde une référence vers rien tant que la clé n'existe pas : l'interface affichera « source indisponible ».
- Le contenu HTML n'est **pas** assaini par le dépôt : l'assainissement (`ammonia`) se fait à l'enregistrement, côté serveur (lot 5).
