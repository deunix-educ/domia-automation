# Sécurité

DomiNet est conçu pour **un réseau domestique que vous maîtrisez**. Cette page dit ce qui est protégé, ce qui est volontairement ouvert sur le LAN, et ce qui ne doit jamais être exposé sur Internet. Pour signaler un problème, voir `SECURITY.md` à la racine du dépôt.

## Ce que la pile de production expose

| Port | Service | Qui y accède | Protection |
|---|---|---|---|
| 443 (80 redirige) | nginx, l'application, Grafana sous `/grafana/` | le LAN | TLS ; l'application demande une connexion ; les seules routes sans authentification sont la page de connexion, la sonde de santé et le téléchargement du certificat (`/dominet-ca.crt`, aussi en HTTP pour qu'un nouvel appareil puisse le récupérer) |
| 1883 | MQTT (Mosquitto) | le LAN | **mot de passe exigé pour chaque client**, droits par compte sur un sujet ; aucun écouteur anonyme |
| 8554 | RTSP (MediaMTX) | le LAN | **un compte est nécessaire** : MediaMTX interroge DomiNet avant chaque lecture ([Accès aux caméras](camera-access.md)) |
| 8189 (udp, tcp) | média WebRTC (MediaMTX) | le LAN | autorisé au démarrage de la vue par le navigateur (jeton), pas sur le média lui-même : ne le redirigez jamais |
| 8080 | interface de Zigbee2MQTT (seulement avec une clé) | le LAN | jeton d'accès (`ZIGBEE_AUTH_TOKEN`) |
| 51820 (udp) et l'interface wg-easy | profil VPN facultatif | Internet si vous le redirigez | clés WireGuard ; l'interface web est protégée par `WG_ADMIN_PASSWORD` (HTTP simple : gardez le port 51821 sur le réseau domestique) |
| 5432 | PostgreSQL | cette machine seulement (`127.0.0.1`) | mot de passe |
| 8181 | Adminer (profil facultatif) | cette machine seulement (`127.0.0.1`) | mot de passe de la base |
| 9997, 9998, 9100, Prometheus, port propre de Grafana | API internes et métriques | le réseau Docker seulement | non publiés (l'API et les métriques de MediaMTX échappent à son authentification pour cette raison) |

## Regarder les caméras

Regarder une caméra en RTSP ou en WebRTC demande un **compte**. Dans le navigateur rien ne change pour vous (un jeton de courte durée, lié à votre session, est envoyé à votre place) ; VLC et les applications du téléphone demandent votre nom d'utilisateur et un mot de passe personnel par appareil, créé dans **Mon compte → Accès aux caméras** et révocable à tout moment. Chaque compte voit toutes les caméras. Détails et conduite à tenir quand un lecteur est refusé : [Accès aux caméras](camera-access.md).

Ce que cela ne fait pas : la vidéo n'est pas chiffrée de bout en bout en RTSP, le média WebRTC est autorisé au démarrage d'une vue et non paquet par paquet, et une personne qui a un compte peut regarder toutes les caméras. Mettez quand même les invités sur un autre réseau ou un autre Wi-Fi, et **ne redirigez jamais les ports 8554 ou 8189 sur votre box**.

## Ce qui est protégé

- **Comptes** : empreintes argon2id, sessions côté serveur (cookie opaque, expiration par inactivité et absolue), protection CSRF, blocage après des échecs répétés (par compte et adresse, par compte, par adresse), un changement de mot de passe ferme toutes les sessions. Mots de passe : de 10 à 128 caractères, pas uniquement des chiffres, différents du nom d'utilisateur.
- **Secrets stockés en base** (mots de passe de caméras, clés SMS) : chiffrés par `DOMINET_MASTER_KEY` (AES-256-GCM). Gardez une copie de cette clé loin de la machine : elle n'est pas dans les sauvegardes.
- **Secrets générés** : `make prod-init` génère chaque mot de passe et jeton ; il n'existe **aucun mot de passe par défaut** et `make prod-guard` refuse une valeur vide ou factice. `.env` est créé en mode 600 et n'est jamais commité (seuls les modèles `.env.example.*` le sont).
- **Conteneurs** : l'application tourne sous un utilisateur non root, les images sont figées à une version, et la base n'est pas joignable depuis l'extérieur de la machine.
- **Serveur MCP** (facultatif, désactivé par défaut) : jetons hachés, limités, révocables, avec expiration obligatoire ; chaque appel est audité ([Audit MCP](mcp-audit.md)).
- **Scripts des automations** : exécutés dans un bac à sable avec une limite d'opérations.
- **Dépendances** : `cargo deny` (licences, sources) et `cargo audit` (vulnérabilités) tournent dans `make ci`. Les avis acceptés, avec leur raison, sont dans `deny.toml`.

## Liste de contrôle pour une installation sûre

1. Installez sur une machine joignable seulement depuis votre LAN ; ne redirigez pas les ports 80, 443, 1883, 8554, 8189 ou 8080 sur la box.
2. Pour l'accès à distance, utilisez le profil WireGuard (un seul port UDP) ou votre propre VPN, avec un `WG_ADMIN_PASSWORD` robuste (et ne redirigez pas le port 51821).
3. Gardez `.env` et les archives de migration privés (ils contiennent des clés) ; rangez `DOMINET_MASTER_KEY` dans un gestionnaire de mots de passe.
4. Installez l'autorité de certification sur vos appareils (`/dominet-ca.crt`) plutôt que de passer outre les avertissements du navigateur.
5. Mettez à jour régulièrement avec `make prod-update` ; lisez les notes de version (`CHANGELOG.md`).
6. Lancez `make prod-doctor` après chaque changement.
