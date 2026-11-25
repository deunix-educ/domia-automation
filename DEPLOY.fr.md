# DomiNet — Déploiement en production

Installation vierge en **trois commandes** sur une machine x86_64 avec Docker (Compose v2.24 ou plus récent) :

```bash
make prod-init      # crée .env : secrets générés, réseau détecté
$EDITOR .env        # relire (courriel, VPN) ; rien d'autre n'est obligatoire
make prod-deploy    # certificats, construction, démarrage, administrateur, données initiales, vérification
```

> Ces cibles déploient **la machine sur laquelle vous les lancez**. Ne les lancez pas sur la machine de développement sans
> projet ni ports décalés (voir « Essai sur une machine de développement »), ni sur l'ancienne production avant la bascule (R8).

## 1. Préparer la machine

- Debian 13 / Ubuntu récent, Docker et le plugin Compose, `git`, `make`, `openssl`, `curl`, `jq` (facultatif).
- `mkcert` recommandé (`sudo apt install mkcert`) : il crée une autorité locale que vous installez sur vos appareils. Sans lui, `make certs` produit un certificat **auto-signé** (avertissement dans les navigateurs, WebRTC et WebSocket fonctionnent quand même une fois accepté).
- Nom d'hôte : `hostnamectl set-hostname dominet` donne `dominet.local` (mDNS : `avahi-daemon`). Sinon `make prod-init` utilise `$(hostname).local` ; vous pouvez imposer `LAN_DOMAIN=mon-nom make prod-init`.
- Le modèle de détection n'est pas dans le dépôt : `make fetch-model` (export officiel Ultralytics dans un conteneur jetable) avant la première construction.
- Dongle Zigbee (facultatif) : branché avant `make prod-init`, il est détecté (`/dev/ttyUSB0` ou `/dev/ttyACM0`) et Zigbee2MQTT est activé.

## 2. Configurer

`make prod-init` copie `.env.example.prod` en `.env` (droits 600, jamais commité), génère les mots de passe de la base, de ReductStore, de MQTT et de Grafana, le jeton Zigbee, détecte le nom et l'adresse de la machine, et affiche le mot de passe Grafana. **Il refuse d'écraser** un `.env` existant.

À relire dans `.env` (voir `.env.example.prod`, qui commente chaque variable, et `docs/configuration.md`) :

| Variable | Rôle |
|---|---|
| `EMAIL_HOST`, `EMAIL_HOST_USER`, `EMAIL_HOST_PASSWORD`, `EMAIL_RECIPIENT`, `DEFAULT_FROM_EMAIL` | alertes par courriel (application **et** Grafana). Sans `EMAIL_RECIPIENT`, les alertes Grafana n'ont pas de destinataire |
| `DYNDNS_HOST`, `WG_ADMIN_PASSWORD` | VPN WireGuard (profil `vpn`), voir §5 |
| `MQTT_USER`, `MQTT_PASSWORD` | compte interne MQTT (DomiNet, Zigbee2MQTT), généré par `prod-init`. **Jamais d'accès anonyme** : chaque client du LAN a son compte (§ 7) |
| `VIDEO_DEVICE`, `RENDER_GID` | décodage vidéo **matériel** (VAAPI, Intel) : écrits par `prod-init` si `/dev/dri/renderD128` existe ; sur une installation existante, `make prod-video` puis `make prod-up`. Sans eux, le décodage est logiciel (environ trois fois plus de processeur par caméra, voir `docs/R6-mesures.md`) |
| `DOMINET_MASTER_KEY` | chiffre les secrets en base (mots de passe de caméras). Générée au premier déploiement si vide. **À copier hors de la machine** : sans elle, une restauration perd ces secrets |

`make prod-guard` (appelé par le déploiement) refuse un `.env` absent, vide ou resté aux valeurs par défaut.

## 3. Déployer

```bash
make prod-deploy
```

Enchaîne : `prod-guard`, `certs` (si `certs/cert.pem` manque), `export-ca`, `prod-up` (construit l'image, démarre la pile et attend que tous les services soient sains), `prod-superuser` (demande le nom et le mot de passe de l'administrateur, ou lit `DOMINET_ADMIN_USER` / `DOMINET_ADMIN_PASSWORD`), `prod-seed` (données initiales, règles d'alerte) et `prod-health` (nginx → application, aucun service en boucle de redémarrage).

Les migrations sont appliquées au démarrage par le service `migrate`. Ouvrir ensuite `https://<LAN_DOMAIN>`.

### Appareils : faire confiance au certificat

Avec mkcert, installez l'autorité sur chaque appareil en ouvrant `http://<LAN_DOMAIN>/dominet-ca.crt` (servi aussi en HTTP exprès, un appareil neuf n'ayant pas encore la CA), puis acceptez-la (Android : Paramètres → Sécurité → Installer un certificat CA).

## 4. Mise à jour

```bash
make prod-update          # sauvegarde, git pull --ff-only, reconstruction, redémarrage, vérification
NOBACKUP=1 make prod-update   # sans sauvegarde      NOPULL=1 : sans git pull
```

La sauvegarde échouée annule la mise à jour. Les migrations sont rejouées par `migrate`.

## 5. Accès extérieur — VPN WireGuard (facultatif)

Renseigner `DYNDNS_HOST` (nom public de la box) et `WG_ADMIN_PASSWORD` (mot de passe de l'interface web de wg-easy, utilisateur `admin`), rediriger UDP 51820 sur la box, puis `make prod-up`. Le profil `vpn` n'est activé que si `WG_ADMIN_PASSWORD` est renseigné. Le VPN est wg-easy 15 (AGPL-3.0) en réseau hôte ; son interface est sur le port 51821, en HTTP simple : gardez-la sur votre réseau domestique et ne redirigez pas ce port.

- **Première configuration.** `make prod-up` règle une seule fois, par l'API de wg-easy, les règles du pare-feu (`iptables-nft`), l'interface réseau utilisée pour la traduction d'adresses (celle par défaut de la machine) et le keepalive (25 s). Ensuite les réglages vous appartiennent : modifiez-les dans l'interface de wg-easy, `make prod-up` n'y touche plus (`tools/prod.sh wg-configure` rejoue la vérification).
- **Lus une seule fois.** Le mot de passe administrateur, les adresses du tunnel (`WG_SUBNET`, `WG_SUBNET6`), le DNS (`WG_DEFAULT_DNS`) et les routes envoyées aux appareils (`WG_ALLOWED_IPS`) ne sont lus qu'au premier démarrage de wg-easy. Changez-les ensuite dans son interface.
- **Appareils.** Créez un appareil par téléphone ou ordinateur dans l'interface, puis scannez son code QR ou importez son fichier dans l'application WireGuard.
- **Mise à jour d'une installation faite pour wg-easy 14.** `WG_ADMIN_HASH` n'est plus utilisé. Le premier `make prod-up` génère `WG_ADMIN_PASSWORD` dans `.env` (`grep '^WG_ADMIN_PASSWORD' .env`) et démarre wg-easy 15 sur un volume neuf et vide (`wg-easy15-data`) : les appareils créés avec la 14 sont à recréer. L'ancien volume `wg-easy-data` reste en place ; vous pouvez le supprimer (`docker volume rm <projet>_wg-easy-data`) quand vous n'en avez plus besoin.

Ajoutez au besoin les adresses du tunnel à `WEBRTC_ADDITIONAL_HOSTS` et à `CERT_EXTRA_SANS` (puis `make certs`).

## 6. Sauvegardes et restauration

Une sauvegarde automatique tourne chaque jour à 03:00 (`BACKUP_HOUR`) : base PostgreSQL (`pg_dump`) et archive du volume ReductStore, sommes SHA-256, 7 jours conservés (`BACKUP_RETENTION_DAYS`), dans le volume `backups-data`. `make prod-backup` en lance une immédiatement. Procédures de restauration complètes : [docs/backups.md](docs/backups.fr.md). Copiez régulièrement le volume `backups-data` **et** `DOMINET_MASTER_KEY` hors de la machine.

## 7. Services et accès

| Accès | Adresse |
|---|---|
| Application | `https://<LAN_DOMAIN>` |
| Documentation | `https://<LAN_DOMAIN>/docs/` (derrière la connexion, FR/EN) |
| Grafana | `https://<LAN_DOMAIN>/grafana/` (lecteur anonyme ; administrateur `admin` / `GRAFANA_PASSWORD`) |
| Flux vidéo WebRTC | `https://<LAN_DOMAIN>/webrtc/` (via nginx) ; RTSP `rtsp://<hôte>:8554/<uuid>` |
| Zigbee2MQTT (profil `zigbee`) | `http://<hôte>:8080`, jeton `ZIGBEE_AUTH_TOKEN` |
| Adminer | `ADMINER=1 make prod-up`, puis `http://127.0.0.1:8181` (tunnel SSH : `ssh -L 8181:127.0.0.1:8181 utilisateur@serveur`) |
| MQTT | port 1883 de l'hôte, **mot de passe obligatoire** ; un compte par client : `make prod-mqtt-user NAME=linky` (affiche le mot de passe une fois ; sujet `linky/#` par défaut), `prod-mqtt-users`, `prod-mqtt-del` |

Prometheus et ReductStore ne sont pas publiés sur l'hôte. Le `/metrics` de DomiNet reste interne.

## 8. Commandes utiles

```bash
make prod-ps                     # état        make prod-logs S=dominet   # journaux d'un service
make prod-logs S=dominet N=100   # les 100 dernières lignes, sans suivre (sans N : suivi en continu, à ne pas combiner avec grep ou tail)
make prod-passwd U=nom           # mot de passe (saisie interactive)
make prod-seed-energy            # suivi des clés du bilan et du tableau Énergie (DRY=1 : simulation)
make prod-dump-context           # instantané YAML de l'installation
make prod-health                 # contrôle de bout en bout
make prod-down                   # arrêt (volumes conservés)
```

## 9. Essai sur une machine de développement

Pour vérifier l'installation sans toucher à rien d'autre : `tools/check-stack.sh <image>` (projet Compose `dominet-chk`, ports décalés, secrets jetables, volumes détruits ensuite ; arrêter la pile de dev avant : `make dev-down`). Pour un vrai déploiement d'essai, utiliser `ENV_FILE`, `PROJECT` et `CERTS_DIR` et décaler les ports dans `.env` (`NGINX_HTTP_PORT`, `NGINX_HTTPS_PORT`, `POSTGRES_HOST_PORT`, `MQTT_LAN_PORT`) ; le port ICE 8189 du WebRTC est fixe (une seule pile à la fois).

## 10. Démo

`cp .env.example.demo .env.demo`, éditer les mots de passe, puis `make demo-up` : application en HTTP sur `:8080`, Grafana `:3001`, une caméra factice `rtsp://mediamtx:8554/demo-cam-1` à déclarer dans l'interface. `make demo-down` l'arrête.

## Différences dev / prod

| | dev (`make dev-up`) | prod (`make prod-deploy`) |
|---|---|---|
| Services | PostgreSQL, ReductStore, Mosquitto, MediaMTX | pile complète + nginx |
| DomiNet | lancé depuis les sources (`make dev-run`) | conteneur, utilisateur non-root |
| Ports | loopback, décalés (1xxxx) | nginx 80/443, MQTT LAN, RTSP, ICE |
| Secrets | jetables, fixés par le Makefile | `.env` généré, droits 600 |
