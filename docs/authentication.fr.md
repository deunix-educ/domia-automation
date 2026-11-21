# Authentification

Sessions **côté serveur** (cookie opaque, données en PostgreSQL), mots de passe **argon2id**, deux rôles : `admin` et `user`.
Décisions : [ADR 0004](adr/0004-auth-sessions.md).

## Créer et gérer les comptes

```bash
dominet createsuperuser --username admin [--email moi@exemple.org]   # saisie masquée, demandée deux fois
dominet passwd --username admin                                      # ferme aussi toutes les sessions du compte
DOMINET_PASSWORD='...' dominet createsuperuser --username admin     # sans saisie (scripts, Docker)
```

Politique de mot de passe : 10 à 128 caractères, pas uniquement des chiffres, différent du nom d'utilisateur. Noms
d'utilisateur : lettres, chiffres et `@ . + - _` ; l'unicité ne tient pas compte de la casse.

## Routes

| Route | Rôle |
|---|---|
| `POST /api/auth/login` `{"username","password"}` | ouvre une session, renvoie le compte |
| `POST /api/auth/logout` | ferme la session |
| `GET /api/auth/me` | compte connecté (401 sinon) |
| `POST /api/auth/password` `{"current_password","new_password"}` | change son mot de passe ; **ferme toutes les sessions** |
| `GET /api/users` | liste des comptes (administrateur) |

Les erreurs ont la forme `{"error":{"code","message"}}` ; `Retry-After` accompagne un 429.

## Sécurité

- **Cookie** `dominet_session` : `HttpOnly`, `SameSite=Lax`, `Secure` (sauf `COOKIE_SECURE=false`, HTTP sur le LAN). Expiration
  après `SESSION_IDLE_MINUTES` d'inactivité (480) et au plus tard après `SESSION_MAX_DAYS` jours (14). L'identifiant est
  **renouvelé à la connexion** (anti fixation de session).
- **Blocage anti brute-force** (fenêtre et durée de 15 min) : 5 échecs pour un même compte depuis une même IP, ou 20 pour un
  compte depuis n'importe où, ou 30 depuis une IP. Pendant le blocage le mot de passe n'est même pas vérifié (même s'il est bon).
  Une connexion réussie remet les compteurs du compte à zéro. Les tentatives sont conservées 7 jours.
- **Pas d'énumération de comptes** : nom inconnu, mot de passe faux, compte désactivé et blocage se ressemblent ; un calcul
  factice de même coût égalise les temps de réponse.
- **CSRF** : toute requête POST/PUT/PATCH/DELETE doit venir de l'origine du site (en-tête `Origin` ou `Referer` comparé à `Host`)
  ou d'une origine de `CSRF_TRUSTED_ORIGINS` ; `Sec-Fetch-Site: cross-site` est refusé. Les clients hors navigateur (curl,
  application Android) ne sont pas concernés.
- **IP du client** derrière nginx : `X-Forwarded-For` n'est cru que si le pair figure dans `TRUSTED_PROXIES`.
- **En-têtes** : `X-Content-Type-Options`, `X-Frame-Options: DENY`, `Referrer-Policy`, `Permissions-Policy`, `HSTS` en TLS,
  `Cache-Control: no-store` sur `/api`. La `Content-Security-Policy` arrive avec le front Leptos (lots 7-8).
- Les mots de passe ne sont jamais journalisés ni affichés (`Debug` des requêtes et de la configuration masqué).
