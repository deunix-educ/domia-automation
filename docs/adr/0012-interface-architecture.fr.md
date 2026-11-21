# 0012 — Interface : fonctions serveur Leptos et découpage en trois crates


- Statut : accepté
- Date : 2026-11-01

## Contexte
Le front est rendu côté serveur puis hydraté en WASM (ADR 0007). Il faut décider (1) comment les pages parlent au
serveur et (2) comment organiser les crates, sachant que les fonctions serveur Leptos doivent exister dans le crate
compilé pour le navigateur (pour fournir le stub client) tout en exécutant du code serveur (état, base, session).

## Options
1. **Le navigateur appelle l'API JSON `/api/*` en `fetch`.** Un seul contrat pour tous les clients, mais le rendu côté
   serveur doit quand même récupérer ses données par un autre chemin (appeler l'API depuis lui-même, ou des fonctions serveur).
2. **Fonctions serveur Leptos** (`#[server]`) : typées de bout en bout, le même code sert le rendu serveur et le navigateur,
   la session et l'état de l'application sont disponibles directement. L'API JSON reste pour l'application Android, les
   scripts et les tests.
3. Découpage : (a) tout dans `dominet-server` (cycle de dépendance avec l'interface), (b) **`dominet-server` en bibliothèque,
   `dominet-web` pour l'interface (dépend du serveur en `ssr`), `dominet` pour le binaire** — retenu.

## Décision
- **Fonctions serveur** pour l'interface, préfixe `/sf/…` (hors de `/api`, pour ne pas confondre les deux contrats). L'API
  JSON `/api/*` est conservée et testée.
- **Trois crates** : `dominet-server` (bibliothèque : config, état, auth, réglages, API, CLI), `dominet-web` (composants, pages,
  fonctions serveur, i18n ; feature `ssr` côté serveur, `hydrate` côté WASM) et `dominet` (binaire fin : assemble les deux).
  Il n'y a ainsi aucune dépendance circulaire, et `dominet-web` peut utiliser `CurrentUser`, `AppState`, etc.
- **Montage** : `dominet-web::ssr::ui_service` produit un service Axum (pages, fonctions serveur, fichiers `/pkg`) monté en
  `fallback_service` du routeur principal. Les middlewares (session, contrôle d'origine, en-têtes de sécurité, identifiant de
  requête) l'enveloppent donc **comme l'API** ; une fonction serveur est protégée contre le CSRF comme n'importe quel POST.
- **État dans les fonctions serveur** : `AppState` est fourni par le contexte Leptos ; l'authentification passe par les mêmes
  extracteurs que l'API (`CurrentUser`, `RequireAdmin`) via `leptos_axum::extract_with_state`.
- **Configuration** : `LeptosOptions` est construit par le code (`SITE_ROOT`, adresse d'écoute), sans variables `LEPTOS_*`.
  Pas de rechargement à chaud en R1 : `make front` puis redémarrage.
- **Build** : `cargo leptos build` depuis la racine (`[[workspace.metadata.leptos]]`) ; CSS repris de `main.css` du projet de référence.

## Conséquences
- Une règle métier utilisée par l'interface et par l'API vit dans la couche service (déjà le cas : `AuthService`,
  `SettingsService`) ; les routes JSON et les fonctions serveur ne sont que de fines enveloppes.
- Les fonctions serveur ne sont pas un contrat public : elles évoluent avec l'interface. Tout client tiers passe par `/api`.
- Les tests d'intégration du routeur complet sont dans le crate `dominet` (`tests/ui.rs`) ; l'hydratation se vérifie dans un
  vrai navigateur (`tests/e2e/`).
- Licences/avis : `BSL-1.0` (`xxhash-rust`) autorisée ; l'avis « non maintenu » sur `paste` (macro de compilation de Leptos)
  est ignoré explicitement dans `deny.toml`, à réexaminer à chaque montée de Leptos.

## Pièges relevés
- `#[server]` exige `prefix` et `endpoint` explicites pour des URL stables ; les accents des textes de la page sont encodés en
  UTF-8 dans le HTML rendu.
- Le fichier WASM est chargé **avant** d'être instancié : un test qui clique tout de suite perd le premier clic ; attendre
  qu'un clic soit pris en compte avant de compter (voir `tests/e2e/home.py`).
- Un test d'intégration qui rend Leptos s'est bloqué une fois en suite complète sans pouvoir être reproduit (40 exécutions
  ensuite, 60 requêtes simultanées sur le vrai serveur sans problème) : les tests ont désormais un délai de 20 s par requête.
