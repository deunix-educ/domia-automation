# 0002 — Stockage IoT : ReductStore (brut) + PostgreSQL (agrégats) (spike 2)


- Statut : accepté
- Date : 2026-10-31
- Verdict du spike : **GO**

## Contexte
ROADMAP §3.2 : l'IoT brut va dans ReductStore (bucket `iot`), les agrégats et l'état récent dans
PostgreSQL. ReductStore n'est pas une base de séries temporelles (pas d'agrégation). Le spike
(`spike (code non conservé)`, `reduct-rs` 1.21.0 contre le serveur 1.21.5, EliteDesk i5-6500) valide la
modélisation des étiquettes, les performances sur **1 M d'enregistrements** et la rétention FIFO.

## Modèle testé
Bucket `iot`, une entrée `dev_{device_id}`, un enregistrement par changement d'état :
corps = JSON de l'état complet (≈ 100 octets), étiquettes = une par clé numérique suivie
(`temperature`, `humidity`, `battery`, `linkquality`), horodatage en µs. Jeu : 100 devices × 10 000 points (1 pt/10 s).

## Mesures
| Mesure | Résultat |
|---|---|
| Ingestion, 1 M enreg. avec 4 étiquettes, 8 flux parallèles, lots de 50 | 595 s = **1 680 enreg./s** |
| Écriture unitaire (`write_record`) | 818 enreg./s (1,2 ms) |
| Stockage | **232 octets/enreg.** (227 Mo sur disque pour 1 M) |
| Dernière heure d'un device (≈ 300 enreg., corps lus) | 11 ms |
| Historique complet d'un device (10 000 enreg.), en-têtes seuls / corps lus | 367 ms / 1,1 s |
| `temperature > 28` sur un device (étiquettes, 10 000 → 2 312) | 99 ms |
| `temperature > 28 ET battery < 90` | 67 ms |
| `temperature > 29.5` sur **les 100 entrées** (1 M scannés → 100 394) | 3,7 s |
| Dernière valeur d'un device (`read_record` sans horodatage) | 0,6 ms |
| Moyenne horaire sur 24 h, calculée côté client (8 600 points) | 0,93 s |
| Quota FIFO 10 Mo, 30 000 écritures de 1 Ko | taille bornée à 10,4 Mo, plus anciens évincés **par blocs** |

Les comparaisons numériques (`$gt`, `$lt`, `$and`) fonctionnent sur des étiquettes stockées en chaîne.

## Décision
- **Bucket `iot`** comme ci-dessus ; une étiquette par clé de `tracked_keys` numérique. Écriture **unitaire** par message (le débit réel de ≈ 50–100 msg/s est ≪ 818/s) ; lots optionnels, plafonnés à **50 enregistrements** (voir pièges).
- **Rôles** : ReductStore = archive brute, forensique et requêtes à seuil ; **PostgreSQL = tout ce qui alimente graphiques et tableaux de bord** (état récent, `device_state_log`, agrégats énergie). Un agrégat horaire calculé depuis ReductStore (≈ 1 s/jour/device) est trop lent pour un affichage interactif.
- **Rétention** : quota FIFO par bucket, **configurable** (`AppSetting`), défaut proposé **5 Go** pour `iot` (≈ 21 M enregistrements, soit des mois au rythme réel puisqu'on n'écrit que les changements de `tracked_keys`). `device_state_log` (PostgreSQL) : rétention en jours, configurable, défaut proposé **30 jours** ; les agrégats journaliers sont conservés sans limite.
- `max_block_size` laissé à la valeur par défaut pour `iot` ; réglé à 1 Mo pour les petits buckets de test.

## Conséquences
- Une requête multi-entrées à seuil (« toutes les sondes > X ») coûte quelques secondes à 1 M d'enregistrements : acceptable pour de l'investigation, pas pour une page temps réel.
- Le quota FIFO évince par blocs, donc la taille oscille autour du quota (≈ ±1 bloc) ; l'espace disque réel doit prévoir une marge.
- Les clés numériques absentes d'un message n'ont pas d'étiquette : les requêtes conditionnelles les ignorent (comportement à tester en R3 avec les fixtures Z2M réelles).

## Pièges relevés
- **Taille d'un lot** : les métadonnées de lot voyagent dans les en-têtes HTTP ; au-delà de ≈ 50–100 enregistrements, le serveur répond `RequestHeaderFieldsTooLarge` (même sans étiquettes à 200). `write_record_batch` (v2) accepte des enregistrements de plusieurs entrées.
- **Suppression de bucket asynchrone** : juste après un `DELETE`, les écritures échouent avec « Bucket is being deleted » ; relancer (le spike retente).
- `BucketInfo` n'expose pas de nombre d'enregistrements : l'obtenir en sommant `EntryInfo.record_count`. `Bucket` n'est pas `Clone` (le partager via `Arc`).
- `RS_API_TOKEN` non vide obligatoire en 1.21 (voir `PROJECT_CONTEXT.md`) ; l'image n'a ni `wget` ni `curl`.
- Non couvert (R6) : buckets `video`/`snapshots`/`detections` (enregistrements de plusieurs Mo, lecture en flux).
