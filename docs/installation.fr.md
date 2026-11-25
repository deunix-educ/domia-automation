# Installation neuve sur l'EliteDesk — pas à pas

Procédure de l'installation de production. **Vous êtes aux commandes** ; chaque étape dit ce qu'elle fait et comment la vérifier. Rien n'est supprimé : les conteneurs et volumes d'une autre installation, s'il en reste, ne sont pas touchés (projet Compose `dominet`). Durée : environ 1 h, dont 15 à 20 min de construction de l'image.

## 0. Avant de commencer
- Une session sur l'EliteDesk (SSH ou écran), avec un compte du groupe `docker`.
- **Dongle Zigbee branché** (il est détecté à `make prod-init`). Aucun autre processus ne doit le tenir.
- L'**autorité mkcert** d'origine, si vous voulez que vos appareils n'installent rien : copiez `rootCA.pem` et `rootCA-key.pem` dans le dossier donné par `mkcert -CAROOT` (créé par `mkcert -install` au besoin) **avant** l'étape 4. Sinon une nouvelle autorité sera créée et chaque appareil installera `/dominet-ca.crt` une fois.
- Un papier ou un gestionnaire de mots de passe pour : le mot de passe de l'administrateur, celui de Grafana, `DOMINET_MASTER_KEY`.

## 1. Récupérer le code
```bash
git clone git@github.com:deunix-educ/domia-automation.git && cd domia-automation
git checkout r8-termine 2>/dev/null || git checkout main      # le tag existera en fin de phase
```

## 2. Vérifier la machine (lecture seule)
```bash
make preflight
```
Il contrôle l'architecture, Docker et Compose (2.24 ou plus), l'espace disque, **les ports 80, 443, 1883, 8554, 8189 (TCP et UDP) et 5432 (boucle locale)**, le dongle, la GPU, mkcert et son autorité, la résolution du nom `.local`, et **liste** (sans y toucher) les conteneurs et volumes existants. Aucun `BLOQUANT` ne doit rester ; les `ATTENTION` se lisent et se jugent. Un port pris par un autre service se règle d'abord (arrêter ce service, ou changer `NGINX_HTTP_PORT`, `NGINX_HTTPS_PORT`, `MQTT_LAN_PORT` dans `.env` à l'étape 3 — le port ICE 8189 et RTSP 8554 sont fixes).

## 3. Initialiser et relire la configuration
```bash
make prod-init       # crée .env (droits 600) : secrets générés, nom et adresse détectés, dongle détecté
$EDITOR .env         # à relire : EMAIL_* et EMAIL_RECIPIENT (alertes), LAN_DOMAIN, éventuellement DYNDNS_HOST + WG_ADMIN_PASSWORD (VPN)
make prod-guard      # refuse un .env vide ou resté aux valeurs par défaut
```
`make prod-init` affiche le mot de passe Grafana : notez-le. **Il refuse d'écraser un `.env` existant.** Le nom d'hôte détecté doit être celui que vos appareils utilisent (`LAN_DOMAIN`).

## 4. Déployer
```bash
make prod-deploy     # certificat, construction, démarrage, administrateur (saisie), données initiales, vérification
```
L'étape demande le nom et le mot de passe de l'administrateur. À la fin : `DomiNet est prêt : https://<LAN_DOMAIN>`.
**Tout de suite après** : copiez `DOMINET_MASTER_KEY` (dans `.env`) hors de la machine. Sans elle, une restauration perd les mots de passe de caméras chiffrés.

Vérifications : `make prod-ps` (tous « healthy », `migrate` et `init` terminés), ouvrir `https://<LAN_DOMAIN>` et se connecter, `https://<LAN_DOMAIN>/docs/` (documentation), `https://<LAN_DOMAIN>/grafana/` (lecteur anonyme ; `admin` + mot de passe de l'étape 3).

## 5. Appareils : faire confiance au certificat
Sur chaque appareil qui ne connaît pas encore l'autorité, ouvrir `http://<LAN_DOMAIN>/dominet-ca.crt` et l'installer (Android : Paramètres → Sécurité → Installer un certificat CA). Avec l'autorité d'origine réutilisée (étape 0), il n'y a rien à faire.

## 6. Zigbee2MQTT
Avec le dongle détecté, Zigbee2MQTT démarre seul (profil `zigbee`, interface sur `http://<hôte>:8080`, jeton `ZIGBEE_AUTH_TOKEN` du `.env`). Si le dossier `deploy/zigbee2mqtt/` contenait déjà un appairage, il est conservé ; sinon, appairez vos appareils (permit join). Ils apparaissent ensuite dans **Automatisation → Appareils** (rôle MQTT de DomiNet).

## 7. Clients MQTT du LAN
MQTT n'accepte **aucun accès anonyme**. Chaque client externe reçoit son compte :
```bash
make prod-mqtt-user NAME=linky       # affiche le mot de passe UNE fois ; sujet linky/# par défaut
make prod-mqtt-users                 # liste les comptes et leurs droits
```
Dans le client : hôte `LAN_DOMAIN`, port 1883 (ou `MQTT_LAN_PORT`), l'utilisateur et le mot de passe. SolarMax est lu par DomiNet lui-même (réglage `SOLARMAX_HOST`), il n'a pas besoin de compte.

## 8. Caméras, scènes, automations, alertes
- **Caméras** : Administration → Caméras, ou Surveillance → Découverte (ONVIF sur le LAN) ; saisir les identifiants des caméras.
- **Scènes, automations, règle d'alerte** : à recréer dans l'interface, depuis votre propre configuration. `make prod-dump-context` produit l'instantané YAML de la nouvelle installation pour comparer.
- **Android** : Android → ajouter l'appareil, jeton affiché une fois, à saisir dans l'application.
- **Réglages** : courriel (SMTP) et SMS (Free Mobile) dans Réglages ; `dominet notify-test` (vous seul) pour un envoi réel.

## 9. Vérifier le résultat
Ouvrir l'application, se connecter, vérifier le tableau de bord et les pages d'**Exploitation** (`make prod-health` fait la même vérification de bout en bout).

## 10. En cas de problème
| Symptôme | Que faire |
|---|---|
| `prod-deploy` s'arrête | lire le message ; `make prod-logs S=<service>` ; corriger ; relancer `make prod-deploy` (rejouable) |
| `additional privileges requested: pass "--allow=network.host"` à la construction | ancienne version du dépôt (Compose 5.6+ refuse `network: host` dans `build`) : `git pull`, puis relancer `make prod-deploy` |
| Un service redémarre en boucle | `make prod-ps`, puis `make prod-logs S=<service>` |
| Page inaccessible | `curl -k https://127.0.0.1/healthz` sur la machine ; `make prod-logs S=nginx` ; vérifier `ALLOWED_HOSTS` et `CSRF_TRUSTED_ORIGINS` |
| Le bouton « Se connecter » ne réagit pas (la page s'affiche, la navigation privée ou un autre navigateur fonctionne) | l'interface n'a pas démarré dans cet onglet. Dans l'ordre : `Ctrl+Maj+R` ; **Brave** : icône du lion → désactiver Shields pour ce site (sans risque : le site ne charge aucun contenu tiers), se connecter, Shields peut ensuite être réactivé ; **Firefox** : vider le cache du site (cadenas → Effacer les cookies et les données de site) |
| Firefox refuse le certificat alors qu'un autre navigateur l'accepte | Firefox a son propre magasin : Paramètres → Vie privée et sécurité → Certificats → Afficher les certificats → Autorités → Importer `dominet-ca.crt` (téléchargé sur `http://<LAN_DOMAIN>/dominet-ca.crt`) |
| Tout recommencer proprement | `make prod-down` (volumes conservés) ; pour effacer **la pile Rust seulement** : `docker compose -p dominet --env-file .env -f deploy/docker-compose.yml -f deploy/docker-compose.prod.yml down -v` — jamais `docker volume prune`, qui toucherait d'autres piles |

## 11. Après l'installation : 48 h de surveillance
Deux fois par jour (matin et soir) :
```bash
make prod-report                 # bilan en LECTURE SEULE : services, ressources, journaux, appareils, automations, alertes, sauvegardes
HOURS=24 make prod-report        # fenêtre de 24 h au lieu de 12
make prod-logs S=dominet N=100   # les 100 dernières lignes d'un service, sans suivre (sans N : suivi en continu, Ctrl+C pour quitter)
make prod-logs S=dominet SINCE=3h | grep -i zigbee2mqtt   # tout ce qui est arrivé depuis 3 h (préférable à N quand on cherche un événement précis)
```
Le bilan n'écrit rien et n'affiche aucun secret ; collez-le tel quel pour qu'on le relise ensemble. À regarder : aucun service en `redemarrages` croissants ni `unhealthy` ; mémoire et swap stables ; peu d'erreurs répétées ; appareils « sans message » attendus (piles) ; erreurs d'automation expliquées ; sauvegarde de 03:00 présente. Grafana → **DomiNet — Vue générale** donne les courbes. Valeurs de référence : `docs/R8-charge.md`.
