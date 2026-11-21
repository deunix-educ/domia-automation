# Sauvegardes et restauration

## Ce qui est sauvegardé
| Source | Fichier | Contenu |
|---|---|---|
| PostgreSQL | `<BACKUP_DIR>/postgres/dominet_AAAAMMJJ_HHMMSS.sql.gz` | tout : comptes, réglages, appareils et leur historique, scènes, automations, caméras, enregistrements indexés, règles d'alerte… |
| ReductStore | `<BACKUP_DIR>/reductstore/reductstore_AAAAMMJJ_HHMMSS.tar.gz` | volume de données : segments vidéo archivés, snapshots et détections si présents, IoT brut |

Chaque fichier a sa somme de contrôle SHA-256 (`….sha256`, vérifiable par `sha256sum -c`). **À garder à part : la `DOMINET_MASTER_KEY`** — sans elle, les mots de passe de caméras et les clés SMS d'une base restaurée sont illisibles. Ne sont pas sauvegardés : segments encore sur disque avant archivage, snapshots locaux, images annotées, tableaux Grafana générés, volumes Prometheus et Grafana (données recalculables ou de courte durée).

## Sauvegarde automatique
Chaque jour à `BACKUP_HOUR` (3 h, heure locale), si `BACKUP_DIR` est renseigné (rôle `scheduler`). Les sauvegardes de plus de `BACKUP_RETENTION_DAYS` jours (7) sont supprimées ensuite ; **la plus récente de chaque nature reste toujours**. Un échec est journalisé (« sauvegarde echouee »), compté (`dominet_backup_runs_total{result="error"}`) et déclenche l'alerte `BackupFailing` ; l'alerte `BackupTooOld` prévient quand aucune sauvegarde n'a réussi depuis 36 h.

## À la main
```bash
dominet backup      # sauvegarde immédiate (base + ReductStore), purge des expirées
dominet backups     # liste
```
Dans le déploiement : `make prod-backup` (lot 6).

## Restaurer
**1. La base** (elle **remplace** les données actuelles ; une seule transaction : tout ou rien) :
```bash
dominet restore --latest postgres --yes             # la plus récente
dominet restore /backups/postgres/dominet_20261109_030000.sql.gz --yes
```
La somme de contrôle est vérifiée d'abord. Sans l'outil : `gunzip -c dominet_….sql.gz | psql -v ON_ERROR_STOP=1 --single-transaction` (si le serveur est PostgreSQL 16 et le dump fait par un client 17, retirer la ligne `SET transaction_timeout` : `| grep -v '^SET transaction_timeout'`).

**2. ReductStore** (ReductStore **arrêté**, son dossier de données vide) :
```bash
dominet restore --latest reductstore --to /chemin/du/volume/reductstore
# puis démarrer ReductStore sur ce dossier
```
Un dossier non vide est refusé (`--force` pour passer outre).

**3.** Redémarrer DomiNet, avec la `DOMINET_MASTER_KEY` d'origine.

## Contrôler la procédure
`tools/check-backup.sh` (après `make dev-up` et `make image`) rejoue tout avec l'image Docker : base jetable sauvegardée puis restaurée dans une autre et comparée table par table ; de vrais enregistrements ReductStore sauvegardés, restaurés dans un autre dossier et relus octet par octet par un second ReductStore ; fichier abîmé refusé. [ADR 0054](adr/0054-backups.md).
