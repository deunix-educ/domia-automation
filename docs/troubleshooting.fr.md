# Dépannage

Commencez par le diagnostic, en lecture seule, qui dit quoi corriger :

```bash
make prod-doctor
```

Il vérifie la machine (fichier de configuration, espace disque, certificat), les services (démarrés, en boucle de redémarrage, en mauvaise santé), l'adresse web, puis interroge l'application elle-même (base, migrations, compte administrateur, clé maîtresse, MQTT, ReductStore). Chaque constat est `OK`, `WARNING` (ça marche, mais à savoir) ou `BLOCKING` (quelque chose ne marche pas), avec la commande qui corrige. Le code de sortie est non nul quand quelque chose bloque. La partie application seule : `docker compose exec dominet dominet doctor`.

## Cas courants

| Symptôme | Cause probable | Que faire |
|---|---|---|
| L'installateur s'arrête sur « port 443 déjà utilisé » | Un autre serveur web sur la machine | L'arrêter, ou déplacer les ports de DomiNet dans `.env` (`NGINX_HTTPS_PORT`) |
| Le navigateur avertit du certificat | L'appareil ne connaît pas encore votre autorité locale | Ouvrir une fois `http://<adresse>/dominet-ca.crt` et l'installer |
| `https://<nom>.local` ne se résout pas | Pas de mDNS sur le client ou la machine | Installer `avahi-daemon` sur la machine, ou utiliser l'adresse IP et fixer `LAN_DOMAIN` |
| Les mots de passe de caméras sont refusés | `DOMINET_MASTER_KEY` est vide ou a été remplacée | Restaurer la clé d'origine depuis votre copie ; une nouvelle clé ne relit pas les anciens secrets |
| Le premier démarrage est très long | L'image est construite en local | Normal sur une petite machine (15 à 40 minutes) ; les images publiées l'évitent |
| Un service redémarre en boucle | Voir son journal | `make prod-logs S=<service> N=100` |
| Grafana redémarre et le journal dit `invalid email address for SMTP from_address config` (nginx et l'application l'attendent) | `DEFAULT_FROM_EMAIL` se termine par une adresse IP (par exemple `dominet@192.168.1.104`) | Mettre `DEFAULT_FROM_EMAIL=dominet@dominet.local` dans `.env`, puis `make prod-up`. `make prod-doctor` le signale |
| `password authentication failed for user "dominet"` (le service `migrate` échoue, rien d'autre ne démarre) ; l'installeur dit « un volume de base de données d'une installation précédente existe » | Le volume PostgreSQL a été créé par une installation précédente avec un autre mot de passe ; un nouveau `.env` ne peut pas l'ouvrir | Remettre l'ancien `.env` et lancer `./install.sh --reuse`. Pour repartir de zéro (**efface ces données**) : `make prod-down`, puis `docker volume rm dominet_postgres-data` |
| Aucune donnée des appareils | MQTT ou Zigbee2MQTT | `make prod-doctor`, puis `make prod-logs S=mosquitto N=50` |

## Demander de l'aide

Lancez `make prod-report` (lecture seule : services, ressources, journaux, automations, alertes, sauvegardes) et joignez sa sortie. Elle ne contient aucun mot de passe ; relisez-la tout de même avant de la partager.
