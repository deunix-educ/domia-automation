# À améliorer — interface

Liste tenue par l'utilisateur et par Claude ; une ligne cochée est traitée.

## Présentation des pages (demandé le 2026-11-07)
- [x] **Interlignes et espacements** (passe CSS du 2026-11-08) : paragraphes des cartes, hauteur de ligne, listes `vc-list`. Reste à relire dans le navigateur de l'utilisateur, page par page (tableau de bord, appareils, énergie, Android).
- [x] **Champs qui se superposent ou s'alignent mal** : formulaires du composant générique `crud` sur deux colonnes (écran ≥ 960 px), cases à cocher avec leur aide dessous, groupes de choix encadrés ; formulaires de la messagerie et d'Android passés sur `vc-form-group`. Les champs de `/admin/cameras` sont regroupés par paires.
- [x] **Page « Écrire un message »** (`/messaging/compose`) : champs empilés, canal et destinataires dans des cadres.
- [x] Page de découverte `/surveillance/discovery` : cartes, un champ par ligne (R6 lot 3).

Outil : `SHOT_DIR=… tools/e2e.sh shots` prend des captures de plusieurs pages (liste réglable avec `SHOT_PAGES`, ex. `admin/cameras!add`).

Méthode proposée : une seule passe CSS (`crates/dominet-web/style/main.css`) puis relecture dans le navigateur de l'utilisateur, en version optimisée (`RELEASE=1 make dev-run`). Une page à la fois, captures avant / après si utile.

## Autres
- Le WASM de développement (48 Mo) rend l'interface lente à réagir : utiliser `RELEASE=1 make dev-run`, ou réduire le profil de développement du WASM (opt-level, `wasm-opt`).

## Suites sur la langue (dépôt en anglais d'abord)
- **L'assistant local est en français seulement** : son prompt système, les phrases qu'il détecte (« voulez-vous continuer ? »), les textes des cartes d'action et les marqueurs de résultats d'outils sont en français volontairement (l'assistant répond en français). Le généraliser demande de les traduire, de répondre dans la langue de l'utilisateur et de rejouer le jeu d'évaluation (`make agent-eval`).
- **Les identifiants de portée MCP sont en français** (`lecture`, `pilotage` ; `configuration` et `administration` sont communs) : ils sont stockés dans les jetons. Les renommer en `read` / `control` demande une migration de données avec alias acceptés.
- **Les textes de notification** (e-mails et SMS d'alerte, messages d'appareils) sont en anglais ; une langue par compte n'est pas implémentée.
- **Les commentaires du code** sont en grande partie en français (plusieurs milliers de lignes) : ils se traduisent petit à petit ; le nouveau code est commenté en anglais.
- **Les données d'exemple des schémas de configuration** (noms comme « Fermeture du soir ») et les alias français acceptés pour les types de configuration (`regle_d_alerte`…) restent, car ils aident un modèle francophone.

## Suites sur les licences
- **wg-easy** : la version 14 (utilisée pour le VPN facultatif) est non commerciale (CC BY-NC-SA 4.0) et la version 15 est AGPL-3.0, deux points vérifiés à la source le 2026-11-23 ; la version 15 est une réécriture. Passer à la 15 (configuration, commande `wgpw` de `DEPLOY.md`) et tester le profil VPN. Voir [Licences](licenses.md).
- **Relire les étiquettes épinglées** (`reduct/store:v1.21.5` et les autres images marquées « connu ») avant la première publication.
- **Visionnage des caméras sans mot de passe sur le LAN** (RTSP 8554, WebRTC 8189) : ajouter des identifiants par spectateur dans MediaMTX et dans l'application. Voir [Sécurité](security.md).

