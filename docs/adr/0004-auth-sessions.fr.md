# 0004 — Authentification et sessions


- Statut : accepté
- Date : 2026-10-31

## Contexte
Décisions déjà arrêtées avec l'utilisateur (ROADMAP §1) : sessions **côté serveur**
(`tower-sessions`, stockage PostgreSQL), mots de passe **argon2id**, rôles `admin` / `user`, comptes
recréés (`dominet createsuperuser`), pas de reprise de hashes extérieurs. Il reste à fixer le
détail d'implémentation et à traiter un écart de versions découvert pendant R0.

## Écart de versions (vérifié sur crates.io le 2026-10-31)
- `tower-sessions` **0.15.0** (février 2026) ; `argon2` 0.6.0 ; `sqlx` **0.9.0** ; `axum` 0.8.9.
- Le store PostgreSQL officiel `tower-sessions-sqlx-store` 0.15.0 (janvier 2025) dépend de
  `tower-sessions` **0.14** et de `sqlx` **0.8** : incompatible avec `tower-sessions` 0.15 + `sqlx` 0.9.
- `axum-login` 0.18 suit la même branche ; non retenu (besoin simple : un extracteur et un contrôle de rôle).

## Options
1. Rester sur `tower-sessions` 0.14 + `sqlx` 0.8 + store officiel — fige `sqlx` en arrière et complique `sqlx-cli`.
2. **Écrire notre propre `SessionStore`** (trait de `tower-sessions-core` 0.15) sur `sqlx` 0.9 — une cinquantaine de lignes, et on maîtrise le schéma.
3. Sessions maison complètes — réinvente cookies signés, rotation d'identifiant, expiration.

## Décision
- Option 2 : `dominet-store::sessions` implémente `SessionStore` (tables `session(id, data jsonb, expiry_date, user_id)`, index sur `expiry_date` et `user_id`). La colonne `user_id` permet d'invalider toutes les sessions d'un compte (changement de mot de passe, désactivation).
- Purge des sessions expirées par une tâche du planificateur (`schedule_job`), pas par un thread dédié.
- Mots de passe : `argon2` (argon2id, paramètres par défaut de la crate relevés si le temps de hachage reste < 500 ms sur l'EliteDesk — à mesurer en R1), sel aléatoire, rehash transparent à la connexion si les paramètres changent.
- Cookie : `HttpOnly; Secure; SameSite=Lax`, identifiant régénéré à la connexion (anti fixation), durée d'inactivité et durée absolue configurables (`AppSetting`).
- Anti brute-force : verrouillage progressif par couple (compte, IP) stocké en base (`login_attempt`), plus limitation de débit par IP au niveau Axum ; messages d'erreur identiques pour compte inconnu / mauvais mot de passe, avec un hachage factice pour égaliser les temps de réponse.
- CSRF : vérification de l'en-tête `Origin`/`Host` sur toute requête non sûre, plus jeton pour les formulaires ; les WebSockets exigent la session et vérifient `Origin`.
- Rôles : `admin` (gestion complète) et `user` (consultation et commandes) ; contrôle par un extracteur Axum `RequireRole`, réutilisé par les server functions Leptos. Les tokens Android restent un mécanisme distinct (UUID par appareil, ADR/R5).
- CLI : `dominet createsuperuser`, `dominet passwd`.

## Conséquences
- Le store maison est à tester contre un vrai PostgreSQL (`testcontainers`) en R1 : création, expiration, suppression, invalidation par utilisateur.
- Surveiller les montées de version de `tower-sessions` ; si le store officiel rattrape `sqlx` 0.9, on pourra revenir dessus.

## Mise en oeuvre (lot 5 de R1, 2026-10-31)

- **Hachage** : argon2id `m = 64 Mio, t = 3, p = 1`, soit ≈ 126 ms par hachage sur l'EliteDesk i5-6500 (mesure en `--release`). Le minimum OWASP (19 Mio, 2 passes) ne coûte que ≈ 23 ms : trop léger pour un serveur personnel. Au plus 4 hachages simultanés (64 Mio chacun) ; condensat remplacé de façon transparente à la connexion si ses paramètres sont anciens.
- **Blocage** : par couple (compte, IP) 5 échecs, par compte 20, par IP 30, fenêtre et durée de 15 minutes. Le seuil par couple seul aurait laissé un attaquant changer d'IP sans limite ; le seuil par compte l'arrête à 20 essais par quart d'heure.
- **Store de sessions maison** (`PgSessionStore`) sur `sqlx` 0.9 ; `tower-sessions-core` 0.15 utilise encore `#[async_trait]`. La colonne `user_id` est dérivée de la clé `user_id` des données.
- **Piège découvert par un test** : `Session::flush()` détruit la session mais **conserve son identifiant en mémoire**, qui est réutilisé à l'enregistrement suivant : la fixation de session restait possible. Il faut enchaîner `flush()` puis `cycle_id()` (voir `auth::session::open`). Le test `login_rotates_the_session_id_against_fixation` verrouille ce comportement.
- **Durée de vie** : inactivité (`SESSION_IDLE_MINUTES`) gérée par la couche de session ; durée absolue (`SESSION_MAX_DAYS`) vérifiée à chaque requête à partir de l'instant de connexion stocké dans la session.
- Détails d'usage : `docs/authentication.md`.
