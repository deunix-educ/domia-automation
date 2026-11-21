# Scènes

Pages, activation et administration des scènes (R4 lot 2). Décisions : [ADR 0031](adr/0031-automation-schema.md) (schéma et domaine), plan R4 (D12 : droits).

Une scène est un ensemble de commandes appliquées ensemble à plusieurs appareils (« Fermeture du soir » : six volets et lampes). Elle a un nom (unique), une icône (emoji ou nom), une description, un emplacement, un état actif / inactif et une liste **ordonnée** de commandes (appareil + objet JSON, par exemple `{"state":"CLOSE"}`).

## Pour tous les comptes
- **`/automation/scenes`** (lien « Scènes » de la page des appareils) : les scènes actives, une carte chacune avec ses commandes dans l'ordre et un bouton **Activer**. Le retour s'affiche à côté du bouton : `✓ Scène activée`, ou `✗` suivi de chaque appareil dont la commande n'est pas partie et de la raison (introuvable ou inactif, refusée, source indisponible).
- **Widget scène** du tableau de bord : le nom, la description et le même bouton. Une scène supprimée ou désactivée est annoncée dans le widget.
- **Activation** = publier chaque commande sur `<préfixe>/<nom de l'appareil>/set`, **dans l'ordre**, en QoS 1. Une commande en échec n'arrête pas les suivantes : les échecs sont rapportés à la fin. Tout compte connecté peut activer (comme la référence : la famille lance « Fermeture du soir »).

## Pour les administrateurs
- **`/admin/scenes`** : liste filtrable, ajout, modification, suppression avec confirmation. L'éditeur de commandes est une liste de lignes (appareil choisi dans une liste, commande JSON), avec **↑ ↓** pour changer l'ordre, **×** pour retirer et « Ajouter une commande ». Les erreurs sont rapportées sur le champ : nom vide ou déjà pris, commande qui n'est pas un objet JSON admis (objet non vide, profondeur et taille bornées), appareil inconnu, plus de 100 commandes.
- **Widgets** (`/admin/widgets`) : la scène d'un widget se choisit dans une liste (plus de numéro à connaître).
- Supprimer une scène supprime ses commandes ; les widgets qui la visaient perdent leur scène, et les automations qui l'activaient échouent (écrit au [journal des automations](automations.fr.md)).

## Comportements notables
- Le **nom d'une scène est unique** (l'API de script active une scène par son nom).
- Une commande vers un appareil **inactif** est rapportée en échec (`introuvable ou inactif`) au lieu d'être publiée.
- Une scène désactivée ne s'active plus (réponse 404).

## Pour les développeurs
- `dominet_server::scenes::{activate, activate_by_id, activate_by_name}` : le moteur d'automations (lot 3) et l'API de script (lot 5) passent par là. Elles s'appuient sur `control::send_command` (refus immédiat sans lien avec le broker).
- Fonctions serveur : `scenes/list`, `scenes/activate` (tout compte connecté), `scenes-admin/{list,save,delete}` (administrateurs).
- Le formulaire utilise le type de champ `FieldKind::Pairs` du CRUD générique (liste de couples choix + texte, valeur du formulaire = JSON `[["12","{…}"],…]`, choix fournis par le contexte `Choices` sous le nom du champ).

## Tests
`cargo test -p dominet --test ui scene` (droits, validation champ par champ, page), `-p dominet-server --test scenes` (publication **dans l'ordre** sur le broker de dev, échec qui n'arrête pas la suite, **les trois scènes de la production** : 6 / 4 / 4 commandes sur les bons topics), `-p dominet-store --test automation` (stockage) ; E2E `scenes` (parcours complet dans Firefox).
