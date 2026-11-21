# 0047 — Pilotage PTZ et bascule jour / nuit


- Statut : accepté
- Date : 2026-11-08

## Contexte
la référence pilote le PTZ par un WebSocket (`PTZConsumer`, ONVIF `ContinuousMove` / `Stop`, presets en base) et commande jour / nuit, IR-cut et lumière par quatre méthodes (OpenIPC, Thingino, ONVIF, Goform). ADR 0008 : le client ONVIF. Plan R6 : D9 (droits), D11 (méthodes testées contre de faux serveurs).

## Décision
- **PTZ ONVIF** (`dominet_surveillance::onvif::ptz`) : `PtzControl` lit une fois l'adresse du service PTZ et le jeton du **premier profil qui porte une configuration PTZ** (celui que le spike 6 a validé sur les caméras), puis `ContinuousMove` (seuls les axes en mouvement sont envoyés, comme la référence ; tout à zéro = un `Stop`), `Stop`, `GetStatus` (position, espace des coordonnées, `MoveStatus`), presets (`GetPresets`, `SetPreset`, `RemovePreset`, `GotoPreset` à mi-vitesse) et `AbsoluteMove` dans **l'espace annoncé par la caméra** (Thingino : degrés ; le défaut serait `[-1, 1]`), avec attente de la fin du mouvement (8 s au plus). Les vitesses sont ramenées dans `0,05..1` (une valeur illisible ne donne jamais un mouvement).
- **WebSocket `/ws/surveillance/ptz/<uuid>/`** : le **protocole de la référence**, inchangé (`move`, `zoom`, `stop`, `get_presets`, `set_preset`, `remove_preset`, `goto_preset` ; réponses `presets` et `error`, plus un `ready` que la référence n'avait pas). Écarts voulus :
  - **administrateurs seulement** (D9) — la référence laissait tout compte connecté ; refus **avant** l'ouverture (session, rôle, origine, caméra inconnue / inactive / sans PTZ) ;
  - **arrêt automatique** d'un mouvement non rafraîchi (`PTZ_DEAD_MAN_SECS`, 8 s) en plus de l'arrêt à la fermeture : un onglet figé ne laisse pas une caméra tourner jusqu'en butée (certaines n'ont aucune butée logicielle, voir ADR 0008) ;
  - un preset **de même nom remplace** l'ancien et **retire l'ancien jeton de la caméra** (la référence le laissait orphelin) ; la base reste la source de vérité ; un preset sans jeton est atteint par `AbsoluteMove` vers sa position enregistrée.
- **Interface** (`/surveillance/live/<uuid>`, administrateurs) : croix de direction (une direction **maintenue** renvoie son ordre toutes les 250 ms, `stop` au relâchement, à la sortie du bouton et si le pointeur est interrompu), zoom, deux vitesses, presets ; liaison reprise toutes les 1,5 s si elle tombe. Les boutons n'apparaissent que si la caméra a un PTZ et une méthode jour / nuit.
- **Jour / nuit** (`dominet_surveillance::night`, trait `NightControl`, quatre méthodes) :
  - `openipc` : `GET /night/<toggle|light>` (`daynight` et `ircut` vont à `toggle`, qui active la bonne broche du moteur) ;
  - `thingino` : `POST /x/json-imp.cgi`, `X-API-Key`, bascule **mémorisée par caméra et par action** (24 h, comme le cache de la référence : la caméra ne relit pas son état) ;
  - `onvif` : `GetImagingSettings` → un seul champ changé → `SetImagingSettings` **de l'objet complet** (un objet partiel remet les autres réglages à zéro) ; `AUTO` et `DAY` vont à `NIGHT`, `NIGHT` à `DAY` ; la lumière bascule la luminosité entre 0 et 80 ;
  - `goform` : JSON `{Type: 1, Data}`, authentification `X-Digest` (défi sans corps, puis réponse MD5 avec `qop=auth`) ; seul `DayNightMode` (ou `Control` pour la lumière) change, `ModeSupportList` n'est pas renvoyé. Identifiants web, à défaut ceux du flux.
  - Commande par la fonction serveur `camera_night` : **administrateurs** (D9 ; la référence : tout compte connecté), caméra active seulement. Aucune erreur ne contient un secret.
- **Vérification** : faux ONVIF **à état** (position, vitesse, presets, réglages d'image, temporisation `MOVING`), faux OpenIPC, faux Thingino (clé d'API), faux Goform (le défi Digest) ; le calcul du Digest est comparé à une valeur **calculée séparément** ; WebSocket réel de bout en bout (droits, origine, vitesses, arrêt automatique, presets, `AbsoluteMove` dans l'espace de la caméra) ; E2E Firefox contre une caméra ONVIF simulée qui journalise chaque ordre (`tests/e2e/ptz.py`).

## Conséquences / limites
- **`goform` n'a pas de caméra de test** (D11) : sa conformité repose sur l'implémentation de référence lu, sur le calcul Digest vérifié et sur un faux serveur ; à confirmer sur un matériel concerné avant la bascule (R8).
- La caméra NVT ne donne jamais sa position (`GetStatus` renvoie 0, 0) : un preset enregistré sur elle a la position 0, 0 et un jeton de caméra (il fonctionne par jeton, pas par position).
- Les essais sur de vraies caméras (PTZ, IR-cut) se feront avec `deploy/.env.cameras` ; aucun ordre n'a été envoyé à une vraie caméra dans ce lot.
