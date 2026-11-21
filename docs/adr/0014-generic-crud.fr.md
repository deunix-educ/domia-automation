# 0014 — Composant CRUD générique des pages d'administration


- Statut : accepté
- Date : 2026-11-01

## Contexte
Le l'administration générée de l'implémentation de référence fournit ≈ 30 écrans CRUD. R1 en remplace quatre (emplacements, navigation, réglages,
utilisateurs) et les phases suivantes ajoutent les caméras, appareils, scènes, etc. Écrire chaque écran à la main
multiplierait le code d'interface, les cas d'erreur et les traductions.

## Options
1. **Un composant générique piloté par une description des champs** (`FieldSpec`), et des fonctions serveur concrètes par
   ressource — retenu.
2. **Tout générique, y compris côté serveur** (une fonction serveur « sauver n'importe quelle ressource » avec des valeurs
   textuelles) — moins de code, mais la validation et les droits passeraient par un contrat faiblement typé, et la table visée
   devient une donnée envoyée par le navigateur.
3. **Une page écrite à la main par ressource** — le plus simple à lire, le plus long à maintenir.
4. **Macro `derive` qui engendre page et fonctions serveur** — élégant, mais coûteux à écrire et à déboguer pour 4 ressources.

## Décision
- `crud.rs` : `CrudTable` (tableau filtrable, formulaire d'ajout/modification, confirmation de suppression sur place,
  erreurs par champ). Une page fournit les `FieldSpec` (nom, clé de traduction, type texte / zone de texte / case à cocher,
  obligatoire, longueur maximale, colonne du tableau), la liste (`Signal<Vec<Row>>`) et **deux `Action`** (enregistrer,
  supprimer) qui appellent ses fonctions serveur. Le composant ne connaît ni le type métier ni le réseau.
- Les fonctions serveur restent **concrètes et typées** par ressource. Chacune commence par `require_admin` (403 sinon) : les
  gardes de pages ne servent qu'à l'affichage. Les refus attendus sont des **valeurs** (`SaveOutcome::{Saved, Invalid, NotFound}`,
  `DeleteOutcome::{Deleted, NotFound, InUse}`), pas des erreurs HTTP ; seules les pannes sont des erreurs (journalisées).
- La validation fait foi côté serveur et rapporte **champ par champ** (`FieldProblem { field, kind }`) pour que l'interface
  traduise le message sous le bon champ. Le domaine porte la règle (`LocationDraft::problems`), `validate` reste pour l'API.
- Les libellés sont des **clés** (`tk!("clé")`, résolues par `I18n::tr` à l'affichage pour suivre la langue). `tk!` rend la clé
  repérable : `tests/i18n.rs` vérifie qu'elle existe en français et en anglais et qu'aucune clé n'est orpheline.
- Contenu affiché toujours échappé par Leptos (test avec `<script>` dans un nom).

## Conséquences
- Types de champ actuels : texte, zone de texte, case à cocher, nombre borné, liste déroulante (7e), mot de passe, colonne d'affichage seul et
  champ non modifiable après création (7f). Un résultat `Blocked` exprime un refus métier (ni erreur ni champ invalide). Les réglages (13 clés fixes, valeurs typées,
  secrets) ne sont **pas** du CRUD : page dédiée sur les mêmes fonctions serveur typées.
- Une nouvelle ressource simple = une description de champs, trois fonctions serveur, une page d'une cinquantaine de lignes.
- Les ressources à relations (liste déroulante vers une autre table), à champ secret ou à règle métier (dernier administrateur)
  ajouteront des types de champs ou des variantes de résultat ; le composant évoluera avec les besoins réels (7e, 7f).
- Pas d'API REST dupliquée pour ces écrans : l'interface passe par les fonctions serveur (ADR 0012).
