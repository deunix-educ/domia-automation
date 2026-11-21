# 0044 — Client MediaMTX et administration des caméras


- Statut : accepté
- Date : 2026-11-07

## Contexte
la référence (`mediamtx_sync`) crée un *path* MediaMTX par caméra active par l'API REST (port 9997) **et** réécrit `mediamtx.yml` pour que la configuration survive à un redémarrage. R6 lot 2 : plan R6 D2 (secrets), D3 (identifiant), D4 (synchronisation), D9 (droits).

## Décision
- **Client `dominet-surveillance::mediamtx`** (reqwest, délai de 5 s) : liste paginée, lecture, `add`, `replace`, `remove`, et `sync` (créer ou remplacer chaque path voulu, supprimer ceux de **nos** caméras qui ne sont plus voulus). Seuls les noms de **32 hexadécimaux** sont les nôtres : le `~.*` de la configuration et tout autre path ne sont jamais touchés.
- **`replace` plutôt que `patch`** : il remplace toute la configuration, donc une option retirée d'une caméra (TCP forcé, enregistrement) disparaît du path. Vérifié sur MediaMTX 1.21.3 : `replace` **crée** le path s'il manque, `delete` d'un path absent répond 404 (traité comme un succès).
- **Pas de réécriture de `mediamtx.yml` (D4)** : l'API seule. Un MediaMTX redémarré perd les paths ajoutés à chaud ; la base fait foi et le serveur web relance une synchronisation complète à son démarrage (et le bouton de l'administration à la demande). En production, le YAML sera généré depuis la base par le déploiement (R7). *Limite connue* : si MediaMTX redémarre seul, ses caméras manquent jusqu'à la prochaine synchronisation.
- **Source = secret** : l'URL de la source contient le mot de passe de la caméra. `PathConf` n'affiche pas sa source dans `Debug`, les erreurs de MediaMTX en sont nettoyées, et rien n'est journalisé.
- **Service `CameraService`** (serveur) : chiffre les secrets avec la donnée associée `camera:<uuid>:<champ>` (l'identifiant public est tiré **avant** l'insertion, car il lie le chiffrement), garde un secret laissé vide à la modification, reprend les identifiants RTSP pour ONVIF s'ils manquent (comme la référence), conserve ce que la synchronisation ONVIF a appris (fabricant, modèle, profil, PTZ). Une camera dont le secret est illisible (clé maîtresse changée) n'est pas supprimée de MediaMTX par erreur.
- **Un échec de MediaMTX ne défait pas l'enregistrement** : la base fait foi, l'échec est journalisé et `sync_all` rattrape.
- **Configuration** : `MEDIAMTX_API_URL`, facultative ; sans elle, rien n'est synchronisé et l'administration le dit.
- **Droits (D9)** : `/surveillance` (consultation) pour tout compte connecte ; `/admin/cameras` (CRUD, synchronisation) et les fonctions serveur correspondantes pour les administrateurs. La page de consultation n'expose ni adresse IP, ni chemin, ni identifiants.
- **Faux MediaMTX** (`dominet_surveillance::testing`, fonction `testing`) : l'API de configuration en mémoire, avec panne simulée, calqué sur le comportement vérifié du vrai. Un test **ignoré par défaut** (`real_mediamtx`) rejoue le client contre le MediaMTX de la stack de dev.

## Conséquences
- Le live, les snapshots, l'ONVIF et le PTZ s'ajoutent aux lots suivants sur ces mêmes caméras.
- Le mot de passe de caméra ne se saisit qu'avec une clé maîtresse configurée.
