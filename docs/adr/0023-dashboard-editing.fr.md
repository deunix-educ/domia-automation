# 0023 — Édition des tableaux : administrateurs seulement, glisser au pointeur, formulaire à champs conditionnels


- Statut : accepté
- Date : 2026-11-02

## Contexte
Le lot 5 de R2 rend les tableaux modifiables : déplacer un widget, créer et corriger des tableaux et des widgets. Le plan disait « propriétaire ou administrateur » pour le
déplacement, alors que la décision D7 et l'implémentation de référence (`is_staff`) réservent la disposition aux administrateurs.

## Décision
- **Édition réservée aux administrateurs** (déplacement, création, modification et suppression de tableaux et de widgets), vérifiée par chaque fonction serveur (403 sinon, testé pour
  les huit). Un compte ordinaire voit et ouvre ses tableaux mais ne les modifie pas ; un administrateur remplit ceux de tous les comptes depuis l'administration. Écart assumé avec le
  plan (« propriétaire ou administrateur ») : permettre à n'importe quel propriétaire de modifier son tableau demanderait un second jeu de droits (champs, types de widgets, Grafana) que
  l'implémentation de référence n'a jamais eu.
- **Déplacement par événements de pointeur** (`drag.rs`) avec capture du pointeur : même code pour la souris, le doigt et le stylet (le glisser-déposer HTML5 ne marche pas au doigt).
  Le widget suit le pointeur par une `transform` posée par la **CSSOM** (autorisée par la CSP, contrairement à l'attribut `style`), un cadre en pointilles montre la case visée, et au relâchement
  la fonction serveur `move_widget` décide : hors grille ou chevauchement → refus, le widget revient à sa place et un message l'explique. **Le serveur a le dernier mot**
  (la contrainte d'exclusion de la base, ADR 0018). Actif seulement avec la grille complète de 12 colonnes (≥ 1201 px) : en dessous, les widgets se placent à la suite et leur position à l'écran n'est
  plus celle de la grille. Pas de déplacement au clavier ; le formulaire de l'administration (colonne, ligne) le remplace.
- **Administration** : `/admin/dashboards` (tableaux de tous les comptes ; propriétaire désigné par son nom d'utilisateur, immuable ensuite) et `/admin/widgets?d=<id>` (widgets d'un tableau). Le composant
  CRUD générique (ADR 0014) gagne `field_visible` : un champ n'est affiché que si une fonction de la page le juge utile selon les valeurs courantes (appareil pour `device` et `grafana`, scène,
  caméra, contenu pour `text`, configuration JSON pour `grafana` et `camera`). La visibilité passe par un `Memo` par champ : saisir dans un champ ne recrée pas les autres.
- **Validation** : la saisie voyage en texte (`WidgetInput`) et le serveur la convertit, champ par champ (`parse_widget`), avant les règles du domaine. Un chevauchement est signalé sur le champ `col`
  (`ProblemKind::Overlap`). Le HTML d'un widget `text` est **assaini à l'enregistrement** (ADR 0022), en plus de l'affichage.

## Pièges rencontrés (à ne pas refaire)
- **Stockage local Leptos dans un composant rendu côté serveur** : `StoredValue::new_local` (donc `SendWrapper`) créé pendant le rendu est libéré par un autre fil → `panic` et page figée.
  Le gestionnaire de glisser (`drag::Handle`) est ordinaire côté serveur et local seulement dans le navigateur.
- **Une ressource dont la source dépend d'une autre ressource** (liste des widgets selon le tableau retenu, lui-même issu de la liste des tableaux) : à l'hydratation, les chargements s'enchaînent dans le désordre
  et la table restait périmée après un enregistrement (échec de 100 % des essais E2E à 1 sur 12 selon la variante). Un seul appel serveur (`widgets_page`) renvoie désormais le tableau retenu et ses widgets.
- **Accolades dans un fichier Fluent** (`{"panel_url": …}`) : ce sont des expressions ; l'erreur d'analyse fait paniquer le chargement des traductions au premier rendu.
- Dans les E2E, attendre la fin de l'hydratation (indicateur « Temps réel : connecté ») avant le premier clic : avant elle, un bouton est présent et actif mais sans effet.

## Conséquences
- Un compte ordinaire ne peut pas peupler son propre tableau : c'est à un administrateur de le faire (ou d'activer plus tard un droit d'édition par compte, hors R2).
- Un tableau ne peut pas changer de propriétaire (supprimer et recréer).
- Les widgets `camera` et `scene` s'enregistrent déjà avec leur référence (UUID, numéro) mais restent des emplacements annoncés tant que R5 et R6 ne sont pas livrés.
