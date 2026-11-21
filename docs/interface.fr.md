# Interface web

Rendu côté serveur (SSR) puis hydratation en WASM, avec Leptos 0.8. Décisions : [ADR 0007](adr/0007-leptos-ssr-frontend.md) et
[ADR 0012](adr/0012-interface-architecture.md).

## Organisation du code

| Crate | Rôle |
|---|---|
| `dominet-server` | bibliothèque : configuration, état, authentification, réglages, API JSON `/api`, CLI |
| `dominet-web` | interface : composants, pages, traductions Fluent, fonctions serveur (`/sf/…`), CSS |
| `dominet` | binaire `dominet` : assemble le serveur et l'interface |
| `dominet-proto` | types partagés serveur/navigateur (DTO) |

## Construire et lancer

```bash
make dev-up && make db-migrate             # base de dev
cargo run -p dominet -- migrate
DOMINET_PASSWORD='un mot de passe solide' cargo run -p dominet -- createsuperuser --username admin
make front                                 # cargo leptos build : WASM + JS + CSS dans target/site, binaire dans target/debug
COOKIE_SECURE=false DATABASE_URL=postgres://dominet:dominet-dev@127.0.0.1:15432/dominet target/debug/dominet serve
# http://127.0.0.1:8000
```

**Piège** : le serveur et le WASM doivent être compilés avec les mêmes réglages, sinon la page s'affiche mais reste inerte (boutons
désactivés). `.cargo/config.toml` les fixe pour toutes les commandes (`--cfg erase_components`, `LEPTOS_OUTPUT_NAME`) ; ne pas les retirer.

`SITE_ROOT` (défaut `target/site`) désigne le dossier des fichiers statiques. Pas de rechargement à chaud : relancer
`make front` puis le serveur après une modification.

## Vérifier

- `make ci` : formatage, clippy (serveur **et** WASM), tests, `cargo deny`, `cargo audit`, métadonnées `sqlx`.
- `crates/dominet/tests/ui.rs` : routeur complet (page rendue FR/EN, 404, en-têtes de sécurité, fonction serveur avec session, CSRF).
- E2E navigateur (hydratation, traductions à chaud, session) :
  ```bash
  firefox --headless --marionette --profile <dossier> -no-remote about:blank &
  python3 tests/e2e/home.py http://127.0.0.1:8000/ admin 'un mot de passe solide'
  python3 tests/e2e/layout.py http://127.0.0.1:8000/ admin 'un mot de passe solide'
  python3 tests/e2e/auth.py http://127.0.0.1:8000/ admin 'un mot de passe solide'
  python3 tests/e2e/locations.py http://127.0.0.1:8000/ admin 'un mot de passe solide'
  python3 tests/e2e/admin.py http://127.0.0.1:8000/ admin 'un mot de passe solide'
  python3 tests/e2e/users.py http://127.0.0.1:8000/ admin 'un mot de passe solide'
  python3 tests/e2e/realtime.py http://127.0.0.1:8000/ admin 'un mot de passe solide'
  SYSTEM_STATS_SECS=1 …   # serveur lancé ainsi pour : python3 tests/e2e/system.py http://127.0.0.1:8000/ admin 'un mot de passe solide'
  ```

## Mise en page

`layout.rs` enveloppe toutes les pages (404 comprise). Pour un compte connecté, la **barre latérale** est dans le HTML
rendu par le serveur (ressource bloquante `layout_data`), donc sans saut à l'hydratation. Le décalage du contenu vient du
CSS (`.vc-sidebar + .vc-main-wrap`), pas d'une classe posée en JavaScript.

- **Navigation dynamique** : `/sf/layout` lit la table `navbar`, retire les entrées `admin_only` pour un compte `user` et
  sépare haut/bas. Un visiteur anonyme reçoit une réponse vide (pas de barre latérale).
- **Icônes Lucide** : générées côté serveur (`crates/dominet-server/src/icons.rs`, partagé avec le serveur MCP) à partir de `data/lucide.tsv`, produit par `tools/gen-lucide.py`
  depuis le `lucide.min.js` officiel (ISC, voir `data/LUCIDE-LICENSE.txt`). Seules les balises graphiques d'une
  liste blanche sont conservées ; un nom inconnu ou invalide donne l'icône `circle-help`, jamais du HTML libre.
- **Repli** mémorisé dans `localStorage` (`vc-sidebar-collapsed`), menu mobile, plein écran, sélecteur de langue,
  déconnexion (`/sf/logout`, qui ferme la session côté serveur).
- **Rendu `SsrMode::Async`** sur les routes : tout est résolu côté serveur avant l'envoi. En streaming hors ordre (le défaut),
  la barre arrivait dans un `<template>` inséré par JS après `</html>` et le contenu glissait de 0 à 220 px (constaté
  dans ~2 chargements sur 12). `ui.rs` vérifie que `<aside>` précède le contenu et qu'il n'y a aucun `<template>`.
- Les tests de `ui.rs` s'exécutent **un par un** (verrou statique) : plusieurs applications Leptos rendues en parallèle dans
  un même processus de test se perturbent (rendu figé ou contexte `AppState` introuvable, ~15 % des exécutions, 0 sur 25
  en série). La concurrence sur une seule application a été vérifiée sur le vrai serveur : 3000 requêtes, 64 en parallèle,
  toutes correctes.
- Le lien « Administration » et les jauges système sont volontairement absents : pages d'administration en 7d–7f,
  statistiques en R2.

## Connexion, gardes et compte

- **`/login`** : formulaire (identifiant, mot de passe) qui appelle la fonction serveur `login`. Les refus attendus sont des
  *valeurs* du résultat (`InvalidCredentials`, `Locked { retry_after_s }`), traduites côté interface ; seule une panne est
  une erreur (journalisée, jamais détaillée au navigateur). Un identifiant inconnu et un mot de passe faux donnent la même
  réponse. Le blocage anti force brute (5 échecs par couple compte+IP, 20 par compte, 30 par IP sur 15 min) s'applique.
  Le paramètre `?next=` est filtré par `nav::safe_next` (chemin interne uniquement) : pas de redirection ouverte.
- **Gardes** : `RequireAuth` enveloppe `/` et `/account`. Pour un navigateur (`Accept: text/html`) un anonyme reçoit une
  vraie réponse **302** vers `/login?next=…` ; la page protégée n'est jamais envoyée. Un compte déjà connecté qui ouvre
  `/login` est renvoyé vers sa cible. Ces gardes servent à l'affichage : **chaque fonction serveur revérifie la session**.
- **`/account`** : identité (nom, identifiant, courriel, rôle, dernière connexion) et changement de mot de passe. L'ancien
  mot de passe est exigé ; la politique (10 à 128 caractères, pas uniquement numérique, différent de l'identifiant) est
  appliquée côté serveur et chaque motif de refus est traduit. Le **même blocage que la connexion** s'applique (une session volée ne permet pas de deviner l'ancien mot de passe) : les vérifications sont journalisées comme des tentatives de connexion. Un changement réussi **ferme toutes les sessions** du
  compte et renvoie vers `/login?notice=password-changed`.
- **Avant l'hydratation** les boutons d'envoi sont inactifs (`use_hydrated`) : sinon une soumission trop rapide partirait
  comme un formulaire HTML ordinaire. Les formulaires ont aussi `method="post"` pour qu'un mot de passe ne finisse jamais
  dans l'adresse.
- **Départ volontaire** (`leave_session`) : déconnexion et changement de mot de passe naviguent vers `/login` et déclarent
  l'état anonyme avant de recharger ; sans cet indicateur la garde de la page quittée redirigeait vers `/login?next=…` et
  la page de connexion renvoyait vers l'accueil.
- E2E : `python3 tests/e2e/auth.py url utilisateur mot_de_passe` (garde, échec puis succès, traduction à chaud, redirection
  ouverte refusée, changement de mot de passe en aller-retour, déconnexion).

## Administration et CRUD générique

Décision : [ADR 0014](adr/0014-generic-crud.md). `crud.rs` fournit `CrudTable` ; une page décrit ses champs (`FieldSpec`),
sa liste et deux actions (enregistrer, supprimer). Pages d'administration : **tiroir « Administration » masquable à droite**
(languette sur le bord droit de l'écran ; visible des seuls administrateurs ; se ferme par la croix, par Échap, par un clic à côté ou par le choix d'une page). Les liens
restent dans le HTML même fermé (`#vc-nav-admin`), seulement cachés.
- **Journaux** : `/admin/logs` (administrateurs) consulte le journal de l'application depuis son dernier démarrage (tampon en mémoire de 20 000 lignes, `dominet_server::logbuf`), les plus récentes d'abord, avec des filtres : niveau minimal, module, texte, période, nombre de lignes, actualisation toutes les 5 s. Les autres services restent dans `make prod-logs`.
- **Tableaux** : une cellule tronquée (colonne étroite) porte la valeur complète dans son `title` (info-bulle) ; « Modifier » et « Ajouter » ramènent au début de la page,
  où s'ouvre le formulaire, même depuis le bas d'une longue liste.

- **`/admin/locations`** : liste filtrable (recherche insensible à la casse), ajout et modification dans un formulaire, suppression
  avec confirmation sur place. Les erreurs de validation arrivent du serveur **champ par champ** et sont traduites sous le champ.
  Un élément supprimé entre-temps donne un message et une liste rafraîchie, pas une erreur.
- **Droits** : `RequireAdmin` affiche, pour un compte ordinaire, une page « Accès refusé » (statut 403, sans aucune donnée) ;
  un anonyme est renvoyé vers la connexion. Chaque fonction serveur (`/sf/locations/{list,save,delete}`) revérifie le rôle.
- E2E : `python3 tests/e2e/locations.py url utilisateur_admin mot_de_passe` (supprime ce qu'il crée).
- **`/admin/navbar`** : même composant, avec les types de champ liste déroulante (position, ouverture) et nombre (ordre), et un texte
  d'aide sous les champs. Validation champ par champ côté serveur (`NavbarDraft::problems`) : icône en minuscules et tirets **et
  existant dans Lucide** (sinon l'entrée afficherait l'icône de repli), URL interne ou `http(s)://` sans espace (`javascript:` refusé),
  cible `_self`/`_blank`, ordre entre 0 et 32767. La barre latérale **se met à jour tout de suite** (rechargement de la navigation après
  chaque écriture). Une entrée interne ouverte dans un nouvel onglet est un lien HTML ordinaire (le routeur ignorerait la cible).
- **`/admin/settings`** : les 17 réglages sont fixes (registre) : pas de tableau CRUD mais une carte par catégorie et un éditeur par type
  (texte, entier, décimal, case à cocher). Une valeur incompatible avec le type est refusée avec un message traduit. **Un secret n'arrive
  jamais au navigateur** : on affiche « Défini / Non défini », le champ est un mot de passe toujours vide, saisir une valeur la remplace,
  « Effacer » la vide ; sans `DOMINET_MASTER_KEY` l'enregistrement d'un secret est refusé (message dédié). Les descriptions viennent du
  registre et restent en français dans les deux langues. Le changement s'applique tout de suite (cache et abonnés du service).
- **Gardes et rechargement** : `RequireAuth`/`RequireAdmin` passent par un composant `Gate` qui lit la ressource *sous* une `Transition`
  (sinon le rendu serveur ne l'attend pas) et ne réagit qu'à un changement de **statut** (anonyme / compte / administrateur) via un mémo.
  Sans cela, recharger la navigation (ce que fait la page Navigation) recréait la page affichée, qui relisait alors la valeur périmée
  sérialisée dans le HTML initial au lieu de recharger. Tous les `Suspense` sont des `Transition` : l'ancien contenu reste affiché pendant
  un rechargement.
- E2E : `python3 tests/e2e/admin.py url utilisateur_admin mot_de_passe` (navigation et réglages ; rétablit tout ce qu'il modifie).

## Utilisateurs (`/admin/users`)

Même composant CRUD, avec trois types de champ de plus : mot de passe (jamais prérempli ni affiché), colonne d'affichage seul (nom
complet, dernière connexion) et champ **non modifiable après la création** (identifiant). Tout est validé côté serveur, champ par champ :
identifiant (1 à 150 caractères, lettres, chiffres et `@ . + - _`, unicité sans tenir compte de la casse), e-mail, mobile (normalisé),
rôle, mot de passe (politique de `Mon compte`).

- **Règle « dernier administrateur »** : elle est appliquée **dans le dépôt** (`update_profile` et `delete`), pas seulement dans l'écran. Une
  transaction prend un verrou consultatif PostgreSQL sur l'ensemble des administrateurs, relit l'état du compte visé et refuse
  (`RepoError::LastAdmin`) toute opération qui laisserait zéro administrateur actif : suppression, désactivation, rétrogradation. Sans ce
  verrou, deux administrateurs qui se rétrogradent en même temps verraient chacun « il en reste un » et passeraient tous les deux (test
  de course dans `repos.rs` et dans `ui.rs`, vérifié en neutralisant le verrou).
- **Protections sur son propre compte** (plus strictes que la règle ci-dessus, volontairement) : un administrateur ne peut pas se supprimer,
  se désactiver, se rétrograder ni changer son mot de passe depuis cet écran (`Mon compte` sert à cela) : sa session serait coupée en pleine
  page. Pour lever cette restriction, retirer les contrôles `Blocked::Yourself` / `Blocked::OwnPassword` de `save_user` et `delete_user`.
- **Effet immédiat** : désactiver un compte coupe sa session en cours (le compte est relu à chaque requête) ; réinitialiser son mot de passe
  ferme toutes ses sessions ; supprimer un compte supprime ses sessions (cascade).
- Les mots de passe ne sont jamais renvoyés ni journalisés (`UserInput` et `NewAccount` masquent le champ dans `Debug`).
- E2E : `python3 tests/e2e/users.py url utilisateur_admin mot_de_passe` (supprime le compte qu'il crée).

## Intégration serveur, CSP et temps réel

Décisions : [ADR 0016](adr/0016-csp-and-websocket-hub.md).

- **CSP** à nonce, posée **pendant le rendu** (`csp.rs`, appelée par `App`) : un nonce différent à chaque réponse, sur les scripts de Leptos et
  dans l'en-tête. Pas de script ni de style en ligne : n'écrire ni `style="…"` ni `on…="…"` dans les vues (utiliser des classes et `on:`). Si
  une fonction a besoin d'une ressource tierce (iframe Grafana, flux caméra), ajouter la directive correspondante dans `csp::policy`.
- **Rendu des pages** : un `GET` dont le rendu ne répond pas en 5 s est abandonné et relancé (3 essais, puis 503 avec `Retry-After`), contournement d'un défaut de Leptos 0.8.23 ([ADR 0021](adr/0021-page-render-guard.md)) ;
  `tools/stress-cold-render.sh` mesure le défaut sur l'image.
- **Statiques** : `/pkg/*` revalidé à chaque chargement (`no-cache`), pages et fonctions serveur en `no-store`, gzip, COOP/CORP `same-origin`.
- **WebSocket `/ws/notifications/`** : session obligatoire, `Origin` contrôlé, battement `ping`/`pong` (`WS_HEARTBEAT_SECS`), **session revalidée à
  chaque battement** (déconnexion, désactivation, suppression ou changement de mot de passe coupent la connexion), 8 connexions par compte au plus.
- **Navigateur** : l'indicateur « Temps réel » de la barre latérale montre l'état (connexion, connecté, hors ligne) ; la connexion se rétablit seule
  avec un délai croissant. Les notifications s'affichent en texte seul. **Réglages → Diffuser un message** (administrateurs) envoie un message à tous
  les utilisateurs connectés : sert aussi à vérifier la chaîne.
- **Abonnements par sujet** ([ADR 0019](adr/0019-multiplexed-websocket-protocol.md)) : sur la même connexion, le client envoie `{"type":"subscribe","topic":"device:7"}`
  (ou `unsubscribe`) ; sujets `notifications` (automatique), `system`, `device:<id>`, `camera:<uuid>` (refusé jusqu'à R6). Le serveur répond `subscribed`
  puis envoie le **dernier état connu** du sujet, ou `refused` (`invalid`, `forbidden`, `limit` à 128 abonnements). Côté navigateur :
  `Realtime::subscribe(&Topic::Device(7))` / `unsubscribe` (comptage de références, rétabli à chaque reconnexion) et `Realtime::device(id)` pour lire l'état.
- **Jauges système** ([ADR 0020](adr/0020-system-statistics.md)) : processeur, mémoire, disque et température dans la barre latérale, alimentées par le
  sujet `system` (mesure toutes les `SYSTEM_STATS_SECS` s). `GET /api/system-stats/` (compte connecté) donne le document décrit ici
  (`cpu`, `memory`, `disk`, `updated_at`) plus `temperature` ; `disk` et `temperature` valent `null` quand la machine ne les expose pas, et l'API répond 503
  tant qu'aucune mesure n'existe. Les barres reçoivent leur largeur par la CSSOM (`style:width`) : la CSP interdit l'attribut `style` rendu.
- **Réglages → Simulateur d'état d'appareil** (administrateurs) : publie un état JSON pour un numéro d'appareil et montre en direct ce que reçoit le navigateur ;
  sert de source tant que MQTT (R3) n'est pas là.
- Étendre : ajouter une variante à `WsServerMsg` (`dominet-proto`) et, pour un nouveau sujet, une variante de `Topic` et une règle dans `authorize`
  (`web/ws.rs`) ; publier avec `state.hub.publish(&sujet, message)` (ou `publish_retained` pour garder le dernier état) ; traiter le message dans `realtime.rs`.
- Tests : `crates/dominet/tests/ws.rs` (vrai serveur et client WebSocket : authentification, origine étrangère, limite, battements, coupure à la
  déconnexion/désactivation, abonnements, dernier état, refus, limite de 128), `ui.rs` (CSP et nonce sur toutes les pages, cache, compression), E2E `python3 tests/e2e/realtime.py url utilisateur_admin
  mot_de_passe ["commande de redémarrage du serveur"]` (script sans nonce bloqué, WebSocket connecté, diffusion, déconnexion, reconnexion après
  redémarrage, simulateur d'état d'appareil, réabonnement après redémarrage).

## Tableau de bord (`/`)

Décisions : [ADR 0022](adr/0022-dashboard-read-view.md). Lecture seule (l'édition arrive au lot 5).

- **Données** : `dashboard_page(d)` (fonction serveur) renvoie les tableaux **du compte** (jamais ceux d'un autre), celui qui est ouvert (`?d=<id>` s'il lui appartient, sinon le
  tableau par défaut, sinon le premier) et ses widgets prêts à afficher : HTML déjà assaini, adresse d'iframe déjà vérifiée, dernier état connu de l'appareil. `create_dashboard(nom)`
  crée un tableau (le premier devient le défaut). Un compte sans tableau voit une invitation à en créer un.
- **Placement** : un widget porte les classes `vc-c-N`, `vc-r-N`, `vc-w-N`, `vc-h-N` (colonne 1–12, ligne 1–100, largeur et hauteur 1–12). **Jamais d'attribut `style`** (CSP). Sur écran moyen
  ou étroit les widgets se placent à la suite. Ligne maximale du domaine : 99.
- **Types** : `text` (HTML assaini par `ammonia`), `grille`, `grafana` (iframe seulement vers l'origine de `GRAFANA_PUBLIC_URL`, sinon « adresse refusée » ; la CSP reçoit `frame-src` de cette
  origine), `device` (voir ci-dessous), `camera` (R6) annoncé, `scene` ([scènes](scenes.fr.md) : nom et bouton d'activation).
- **Appareils** : le widget s'abonne à `device:<id>` (ADR 0019) et suit l'état en direct ; le cadre est vert quand l'appareil est allumé, atténué quand il est indisponible. La présentation demandée
  (`device/light`…) l'emporte ; en automatique elle est déduite des clés de l'état (`dominet_domain::widget_view`). Les commandes (ON/OFF, curseurs, +/−) sont présentes mais **désactivées jusqu'à R3**.
  Avant R3, l'état vient du **simulateur** (Réglages → Simulateur d'état d'appareil).
- **Étendre** : une nouvelle présentation = une variante de `Presentation` (domaine), un cas dans `resolve_presentation` si elle doit être déduite, une fonction `*_view` dans `dashboard.rs`.
- **Édition** ([ADR 0023](adr/0023-dashboard-editing.md)) — **administrateurs seulement** (chaque fonction serveur refuse les autres : 403) :
  - *Déplacer* : glisser l'en-tête d'un widget (souris, doigt ou stylet) sur l'écran large (≥ 1201 px). Un cadre en pointilles montre la case visée ; au relâchement `move_widget` enregistre,
    ou refuse (hors grille, emplacement occupé) et le widget revient avec un message. Persistant (rechargement).
  - *Tableaux* : `/admin/dashboards` (tous les comptes ; propriétaire = nom d'utilisateur, fixé à la création).
  - *Widgets* : `/admin/widgets?d=<id>` ; le formulaire n'affiche que les champs du type choisi (`field_visible` du CRUD générique) ; chevauchement signalé sur « Colonne » ; HTML du widget `text` assaini à
    l'enregistrement ; bouton « Modifier les widgets » sur le tableau.
  - Un compte ordinaire voit ses tableaux sans les modifier : un administrateur les remplit pour lui.
- **Pièges** : pas de `StoredValue::new_local` dans un composant rendu côté serveur (panic, page figée) ; pas de ressource dont la source dépend d'une autre ressource (chargements dans le désordre à
  l'hydratation) ; pas d'accolades `{…}` dans un `.ftl` ; attendre l'hydratation (« Temps réel : connecté ») avant de cliquer dans un E2E.
- Tests : `ui.rs` (invitation, création, placement, assainissement, Grafana et CSP, sélecteur et isolation des comptes, droits d'édition, validation champ par champ, déplacements refusés), `widget_view` et `html` (unitaires), E2E `dashboard.py`
  (prépare ses widgets en SQL via `E2E_DB_SHELL`, posé par `tools/e2e.sh`).

## Traductions

Fluent (`crates/dominet-web/locales/{fr,en}/main.ftl`). Anglais par défaut ; la langue vient du cookie `lang`, puis de
`Accept-Language` ; le sélecteur la change sans rechargement. `<html lang>` est synchronisé.

### Ajouter une langue
Le français et l'anglais sont les langues de référence (documentation, tests E2E) ; une autre langue s'ajoute sans toucher au code :
```bash
make i18n-new LANGUE=es          # crée crates/dominet-web/locales/es/main.ftl depuis l'anglais (FROM=fr pour partir du français)
make i18n-status                 # par langue : messages encore identiques à l'anglais, clés manquantes ou en trop, variables différentes
make i18n-status LIST=1          # ... et la liste des clés à traduire
make front                       # reconstruit l'interface : la langue apparaît dans le sélecteur
```
Traduire chaque valeur **sans toucher aux clés ni aux variables** (`{ $minutes }`) ; les textes que le français partage avec l'anglais (« OK », « Grafana »…) restent tels quels. Le nom affiché dans le sélecteur (« Español ») vient du code : il doit être connu de `leptos-fluent` (`es`, `de`, `it`, `pt-BR`…), sinon la compilation s'arrête en le disant. La langue est ensuite choisie comme les autres : cookie `lang`, puis `Accept-Language` ; une langue inconnue retombe sur le français.

Les traductions sont **compilées dans le binaire** : une nouvelle langue demande une reconstruction de l'image (`make prod-up`). Le test `cargo test -p dominet-web --test i18n`, rejoué par `make ci`, exige que toute langue définisse exactement les clés de l'anglais, avec les mêmes variables : un fichier incomplet ou une variable renommée fait échouer le push. La documentation (ce site) reste en français et en anglais.
