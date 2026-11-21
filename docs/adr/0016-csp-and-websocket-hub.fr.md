# 0016 — CSP à nonce et hub WebSocket de base


- Statut : accepté
- Date : 2026-11-01

## Contexte
Lot 8 de R1 : durcir l'intégration serveur de l'interface (statiques, politique de sécurité du contenu) et poser la base du temps réel
(`/ws/notifications/`) que R2 et la suite (appareils, caméras, Android) étendront.

## Options
**CSP**
1. Balise `<meta http-equiv>` posée par `leptos_meta` (méthode documentée par Leptos) — ignore `frame-ancestors`, `report-uri` et `sandbox`.
2. En-tête posé par un middleware avec son propre nonce — impossible : Leptos génère le nonce de ses scripts pendant le rendu et
   ne permet pas d'en imposer un (le champ est privé).
3. **En-tête posé pendant le rendu** par le composant racine, qui lit le nonce de Leptos (`use_nonce`) et insère l'en-tête dans
   `ResponseOptions` — retenu.
4. `'unsafe-inline'` — refusé : annule l'intérêt de la CSP.

**Temps réel**
1. SSE ou long-polling — pas de canal bidirectionnel, et le contrat Android est en WebSocket (annexe A).
2. **WebSocket Axum + hub `tokio::sync::broadcast` par sujet** — retenu (ROADMAP §R2).
3. Bus externe (Redis) — supprimé du projet (ROADMAP).

## Décision
- **CSP** : `default-src 'self'; script-src 'self' 'nonce-…' 'wasm-unsafe-eval'; style-src 'self'; img-src 'self' data:;
  font-src 'self'; connect-src 'self'; object-src 'none'; base-uri 'self'; form-action 'self'; frame-ancestors 'none'`. Nonce
  différent à chaque réponse. Aucun style en ligne ni gestionnaire `on…=` : le seul `style=` (bouton de réouverture de la barre
  latérale) est devenu une classe. Un test balaie toutes les pages (nonce identique en-tête / scripts, pas d'attribut interdit) ; l'E2E
  vérifie dans Firefox qu'un script injecté sans nonce est bloqué et que le WebSocket passe malgré la politique. Les directives
  `frame-src` (Grafana), `media-src`/`connect-src` (flux caméras) seront ajoutées avec les fonctions concernées.
- **Statiques** : `/pkg/*` en `Cache-Control: no-cache` (revalider à chaque chargement, réponse 304) — les noms de fichiers ne changent pas
  d'une version à l'autre, et un WASM périmé avec un JS neuf casserait l'hydratation ; le nommage par empreinte (`hash-files`) permettrait un
  cache long, il reste une amélioration possible (lot 9, image). Pages et fonctions serveur en `no-store` (privé, pas d'historique).
  Compression gzip (le WASM perd ~70 %). `Cross-Origin-Opener-Policy` et `Cross-Origin-Resource-Policy` : `same-origin`.
- **Hub** (`hub.rs`) : sujets créés à la demande, `broadcast` de 64 messages, un abonné trop lent perd des messages (journalisé) sans
  bloquer les émetteurs ; sujet supprimé sans abonné ; 8 connexions simultanées au plus par compte.
- **`/ws/notifications/`** : session obligatoire (401 sinon, **avant** l'ouverture du WebSocket) ; en-tête `Origin` contrôlé comme pour
  les requêtes non sûres (les navigateurs n'appliquent pas la politique de même origine aux WebSocket : sans cela une page tierce
  pourrait ouvrir la connexion avec le cookie de l'utilisateur) ; messages du navigateur limités à 1 Kio. **Battement**
  (`WS_HEARTBEAT_SECS`, 25 s par défaut) : `ping` du serveur, `pong` attendu avant le suivant (sinon fermeture 4408) ; à chaque
  battement la **session est revalidée** (elle existe, le compte est actif, son mot de passe n'a pas changé) sinon fermeture 4401 :
  déconnexion, désactivation, suppression ou réinitialisation coupent aussi le temps réel, au plus un battement plus tard.
- **Client WASM** (`realtime.rs`) : ouvert tant qu'un compte est connecté ; reconnexion avec délai doublant (1 s à 30 s, +25 % d'aléatoire) ;
  4401 arrête les tentatives ; indicateur d'état dans la barre latérale ; notifications affichées en **texte seulement**.
- **Diffusion** : fonction serveur réservée aux administrateurs (`broadcast_notification`, page Réglages) : utile en soi et seule façon
  de vérifier la chaîne de bout en bout avant l'arrivée des messages MQTT.

## Conséquences
- Format des messages (`WsServerMsg` / `WsClientMsg`, étiquette `type`) dans `dominet-proto` : c'est le contrat à étendre en R2 (`device_state`...).
  Les routes `/ws/devices/`, `/ws/surveillance/…` et `/ws/android/…` (annexe A) seront d'autres routes sur le même hub ; l'authentification
  par jeton de l'application Android remplacera la session pour `/ws/android/device/{token}/`.
- Le hub est en mémoire d'un seul processus (conforme au déploiement mono-serveur) ; une répartition sur plusieurs instances demanderait un
  bus externe.
- La revalidation par battement a un coût (deux lectures en base par connexion et par battement) acceptable à cette échelle ; un événement
  de révocation poussé par le hub pourrait la remplacer plus tard.
- Une politique stricte oblige à ne plus écrire de `style=`/`on…=` en ligne ni à charger de ressources tierces sans modifier la CSP.
