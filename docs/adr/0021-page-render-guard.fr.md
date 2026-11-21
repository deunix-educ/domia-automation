# 0021 — Garde-fou du rendu des pages : abandon et relance d'un rendu figé


- Statut : accepté (contournement, à retirer quand Leptos corrige le défaut)
- Date : 2026-11-02

## Contexte
Le E2E `realtime` échouait environ 1 fois sur 10 sur l'image Docker (page qui ne se termine jamais : `goto` ou `fetch` sans réponse, 30 s de délai du script). La
recherche, résumée :

- **Le serveur n'est pas cassé** : pendant le blocage, `/healthz`, `/` et `/admin/settings` répondent en 0,0 s à un autre client avec le même cookie. Une seule requête
  est perdue.
- **Reproduit sans navigateur** (`tools/stress-cold-render.sh`) : après un redémarrage à froid, 3 requêtes concurrentes sur la même page figent 1 requête dans 1 à 5 %
  des cas (`/admin/settings` comme `/`) ; 0 sur 100 avec une seule requête à la fois, 0 sur 150 sur un point d'accès qui n'interroge que la base (`/api/auth/me`, huit
  requêtes concurrentes), 0 sur 100 quand la même page a déjà été rendue une fois. Image de production, binaire de debug et binaire optimisé : même comportement
  dès que les rendus se chevauchent à froid.
- **Tous les fils du serveur dorment** pendant le blocage (0 tick de processeur sur une seconde) : ce n'est ni une boucle ni un verrou tenu, c'est une attente qui n'est
  jamais réveillée dans le rendu `Suspense`/`Transition` de Leptos 0.8.23 (dernière version stable ; la 0.9 n'existe qu'en candidate).
- **Indépendant de R2** : collecteur de statistiques actif ou presque inactif (1 s ou 3600 s) donne le même taux (2/40 contre 1/40), et le piège « plusieurs applications
  Leptos rendues en parallèle se perturbent » figurait déjà dans `PROJECT_CONTEXT.md` depuis R1 (le test `ui.rs` les sérialise).
- Les `panics` du journal (« reactive value that has already been disposed », dans `tr!` du sélecteur de langue) sont une **conséquence** : à la déconnexion du client, la
  requête est abandonnée, son contexte réactif est libéré et une tâche orpheline du rendu fige se réveille. Ils apparaissent aussi quand un client coupe pendant un rendu sain.
- Hypothèses essayées et écartées : ressource `new_blocking` de la mise en page (remplacée par `new` : blocage inchangé), nettoyages `on_cleanup` des abonnements
  temps réel (la lecture fautive est celle de `leptos-fluent`, pas la nôtre), tri par en-têtes de Firefox (`Accept-Language`…), mode de compilation.

## Options
(a) Attendre un correctif de Leptos — laisse 1 à 5 % de pages figées dans les situations de rendus concurrents à froid.
(b) Passer à la 0.9 candidate — changement d'API non maîtrisé en pleine phase R2.
(c) Réécrire les pages pour éviter `Transition`/ressources — énorme, et la cause exacte n'est pas isolée.
(d) **Garde-fou : borner le rendu d'un `GET` et le relancer** — un `GET` ne modifie rien, un nouveau rendu aboutit presque toujours — retenu.

## Décision
`web/render_guard.rs` enveloppe le service d'interface (pages et fichiers statiques) : pour un `GET`/`HEAD` sans corps, la réponse doit arriver en **5 s** (un rendu
normal prend quelques dizaines de millisecondes) ; sinon le rendu est abandonné et la requête, copiée à l'identique (méthode, adresse, en-têtes, extensions : session, adresse
du pair), est relancée, **3 essais au plus** ; après quoi la réponse est un **503 avec `Retry-After: 1`**. Les `POST` (fonctions serveur), les requêtes avec corps et les
WebSockets ne sont jamais minutés ni relancés. Chaque abandon est journalisé (`WARN … rendu de page sans reponse : abandonne et relance`).

Vérifié par `tools/stress-cold-render.sh` : 300 démarrages à froid sur l'image avec le garde-fou, 0 anomalie, 1 rendu relancé et réussi (sans garde-fou : 1 à 5 requêtes
figées sur 100 démarrages).

## Conséquences
- Un rendu figé coûte 5 s au lieu d'un blocage indéfini ; un rendu réellement lent (> 5 s, base saturée) est relancé inutilement, jusqu'à 3 fois (lecture seule, borné).
- Le journal contient des `panics` de tâches orphelines après un rendu relancé (bruit, sans effet sur les autres requêtes : confirmé par le test de charge).
- À retirer quand Leptos corrige le défaut : supprimer `render_guard`, reconstruire l'image et exiger **0 anomalie et 0 rendu relancé** de `tools/stress-cold-render.sh`
  sur plusieurs centaines de démarrages avant de conclure.
- Le défaut est probablement présent depuis R1 ; les E2E y échappaient parce que les pages étaient rendues une par une.
