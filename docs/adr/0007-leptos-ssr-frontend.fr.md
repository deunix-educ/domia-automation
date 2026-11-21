# 0007 — Front : Leptos SSR + hydratation, WebSocket, i18n Fluent (spike 1)


- Statut : accepté
- Date : 2026-10-31
- Verdict du spike : **GO**

## Contexte
Le front est un seul binaire Axum qui rend du Leptos côté serveur puis hydrate en WASM (pas de SPA
séparée). Le spike valide, de bout en bout : SSR, hydratation, temps réel WebSocket, thème sombre
repris du projet de référence (variables CSS), i18n FR/EN en Fluent. Code : `spike (code non conservé)`
(Leptos 0.8.23, `leptos_axum` 0.8, `leptos-fluent` 0.3.1, `cargo-leptos` 0.3.11, Axum 0.8).

## Résultats
E2E `spike (code non conservé)` (Firefox 153 ESR headless, protocole Marionette) : **16/16**.
- SSR : HTML complet côté serveur, FR par défaut ; `Accept-Language: en` → EN ; le cookie `lang` l'emporte sur l'en-tête.
- Hydratation : compteur interactif après chargement du WASM.
- Temps réel : hub `tokio::sync::broadcast` → `/ws/devices/` (message de l'annexe A.1) ; valeurs et compteur mis à jour sans rechargement ; **reconnexion automatique** côté WASM (retry 1 s, page à nouveau « connecté » 3 s après la relance du serveur).
- i18n : bascule FR/EN à chaud, `<html lang>` synchronisé, titre de page, pluriel Fluent (`[one]`/`*[other]`), langue conservée au rechargement (cookie → SSR).
- Thème : sombre par défaut, bascule clair via `data-theme` sur `<html>` (variables CSS reprises telles quelles).
- Build : incrémental dev ≈ 7 s ; release à froid ≈ 2 min 50 (4 cœurs). WASM release `opt-level = "z"` + LTO + wasm-opt : **448 Ko** (190 Ko gzip), JS 17 Ko, binaire serveur 9,7 Mo.

## Décision
- **Leptos 0.8.x** (0.8.23). La 0.9.0-rc1 est publiée le jour même et `leptos-fluent` 0.3.1 exige `< 0.9` : on réévalue à la sortie de la 0.9 finale et du support `leptos-fluent`.
- `cargo-leptos` pour le build ; un seul crate à features `ssr`/`hydrate` côté `dominet-web`, les types partagés (messages WS) dans `dominet-proto`.
- i18n : `leptos-fluent` avec `locales/{fr,en}/main.ftl`, cookie `lang` + `Accept-Language` côté serveur, **`customise: |b| b.set_use_isolating(false)`** obligatoire.
- Client WebSocket : `web-sys` direct (≈ 40 lignes), avec reconnexion ; heartbeat applicatif et authentification par session à ajouter en R2.
- E2E : Marionette/Firefox sans dépendance suffit au spike ; Playwright (prévu R2/R6/R8) nécessite Node, absent de l'hôte → à trancher en R2.

## Pièges relevés
- Fluent entoure les variables de marques bidi U+2068/U+2069 : visibles dans les tests et les copier-coller → les désactiver (voir ci-dessus).
- `leptos-fluent` n'a pas de feature `hydrate` (seulement `ssr`, `axum`, `actix`…) ; `<html lang>` ne doit pas être écrit à la main dans le `shell` (doublon avec la synchronisation de `leptos-fluent`).
- `cargo leptos serve` ne recompile pas à chaud (`cargo leptos watch` le fait) ; la clé `env` de `[package.metadata.leptos]` est ignorée par cargo-leptos 0.3.11.
- Non couverts ici (à faire en R1/R2) : persistance du thème (`localStorage`), drag-and-drop des widgets, `cargo leptos` en CI/Docker (cache des dépendances).
