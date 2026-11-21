# Déménager vers une autre machine

Une seule archive porte tout : la base, la vidéo et l'historique (ReductStore) et, chiffrés par une phrase de passe, les clés et fichiers que vous perdriez sinon. L'ancienne machine n'est que **lue** (plus une sauvegarde normale) ; vous décidez du moment de la bascule.

## Ce qui est repris, et ce qui ne l'est pas

| Repris | Provenance | Remarques |
|---|---|---|
| Comptes, appareils, règles, automations, scènes, caméras, réglages | sauvegarde de la base | mêmes mots de passe : les empreintes sont restaurées |
| Historique et vidéo archivée | sauvegarde ReductStore | `--skip-reductstore` l'écarte (volumineux) |
| `.env` et `DOMINET_MASTER_KEY` | partie chiffrée | sans la clé, les mots de passe de caméras enregistrés sont illisibles |
| L'autorité mkcert (clé comprise) et le certificat | partie chiffrée | **les appareils continuent de faire confiance à la nouvelle machine**, rien à réinstaller |
| Dossier Zigbee2MQTT | partie chiffrée | l'appairage est conservé, aucun appareil à réappairer |
| Comptes MQTT de vos clients du LAN | partie chiffrée | |
| Clés du VPN, données Grafana | partie chiffrée | Grafana seulement avec `--with-grafana` |

Non repris : les enregistrements en cours (le volume `recordings` contient des fragments pas encore archivés), les journaux des conteneurs et les images Docker elles-mêmes (la nouvelle machine les télécharge ou les construit).

## Étapes

**1. Sur l'ancienne machine** (pile en marche) :

```bash
make prod-export OUT=/chemin/cle-usb      # demande une phrase de passe deux fois
```

Elle lance une sauvegarde normale, copie les plus récentes, rassemble les fichiers, chiffre les clés (AES-256, PBKDF2 600 000 tours) et écrit `dominet-migration-<date>.tar` avec son `.sha256`. Gardez la phrase de passe **à part** de l'archive.

**2. Sur la nouvelle machine** : installez les prérequis (Docker, Compose, `git`, `make`, `openssl`, `curl`, `mkcert` pour l'autorité), faites un `git clone` du dépôt **dans une version au moins aussi récente que l'ancienne**, copiez l'archive, puis essayez sans rien changer :

```bash
make prod-import FILE=/chemin/dominet-migration-….tar DRY=1
```

Elle vérifie l'archive et la phrase de passe, montre le contenu, compare l'architecture, la version majeure de PostgreSQL et la version du schéma, et liste ce qu'elle ferait.

**3. Importer pour de vrai** :

```bash
make prod-import FILE=/chemin/dominet-migration-….tar
```

L'import écrit la configuration (en adaptant l'adresse, le port de la clé Zigbee et le décodage vidéo matériel à cette machine), restaure l'autorité et les certificats (en émettant un nouveau certificat de la même autorité si l'adresse a changé), démarre la pile, restaure les comptes MQTT, la base puis ReductStore, applique les migrations que l'ancienne machine n'avait pas, redémarre et vérifie. Il **refuse d'écraser une installation existante** sauf avec `YES=1` (l'ancien `.env` est alors gardé sous `.env.before-import-<date>`).

**4. Basculer.** Faites pointer le nom que vos appareils utilisent (`LAN_DOMAIN`, par exemple `dominet.local`) vers la nouvelle machine : donnez-lui l'ancien nom d'hôte, ou changez l'entrée de votre routeur. Avec une clé Zigbee, déplacez-la, renseignez `ZIGBEE_DEVICE` dans `.env` si l'import ne l'a pas trouvée, puis `make prod-up`. Vérifiez avec `make prod-doctor`.

## Entre architectures et versions

- amd64 vers arm64 (et retour) est permis par les formats de données : la base est un export logique, les fichiers de ReductStore ne dépendent pas du processeur. Seule l'**image** change, et l'import utilise celle de la machine où il tourne. Non testé sur du vrai matériel arm64 (voir [Machines prises en charge](machines.md)).
- La version majeure de PostgreSQL doit correspondre (l'import refuse sinon). Un code plus récent que l'archive convient : les migrations manquantes sont appliquées. Un code plus **ancien** que l'archive est refusé : mettez d'abord le dépôt à jour.

## Répétition

`make check-migrate` joue toute la procédure sur deux piles jetables d'une même machine : administrateur, données, un enregistrement ReductStore, un compte MQTT et des fichiers Zigbee sont exportés, la source est détruite, une cible importe, et tout est comparé (comptes et empreintes de mots de passe, lignes, enregistrement, clés, autorité, signature du certificat, comptes, fichiers). Il vérifie aussi les refus (phrase de passe fausse, archive abîmée, installation existante, essai à blanc). Arrêtez d'abord la pile de dev (`make dev-down`), relancez-la après (`make dev-up`).

Avant de compter sur un déménagement, répétez-le avec vos propres données : **ancienne machine → nouvelle machine, d'abord en essai à blanc, puis pour de vrai sur la nouvelle pendant que l'ancienne continue de tourner** ; ne basculez le nom qu'ensuite.

## Limites

- La phrase de passe protège les clés contre le vol de l'archive ; elle n'authentifie pas l'archive face à quelqu'un qui peut la modifier. Gardez-la sous votre responsabilité.
- L'archive contient vos données : vidéo et historique en clair. Chiffrez l'endroit où vous la rangez.
- Une restauration de base remplace les données de la cible ; il n'y a pas de fusion.
