#!/usr/bin/env bash
# SPDX-License-Identifier: AGPL-3.0-or-later
#
# Controle de bout en bout des sauvegardes (R7 lot 4) avec l'IMAGE Docker (`dominet:dev`, client PostgreSQL 17) contre la PostgreSQL de dev
# (16) et de VRAIS ReductStore jetables :
#  - base jetable -> `dominet backup` -> `dominet restore` dans une AUTRE base -> comparaison table par table (la ligne
#    `SET transaction_timeout` de pg_dump 17, inconnue du serveur 16, est ecartee par la restauration) ;
#  - ReductStore : des enregistrements ecrits par l'API, sauvegardes (archive du volume), restaures dans un autre dossier, relus par un
#    SECOND ReductStore et compares octet par octet ;
#  - fichier abime : la restauration refuse avant de toucher a la base.
# Rien n'est envoye a l'exterieur, aucune production n'est touchee. Usage : tools/check-backup.sh (stack de dev : make dev-up ; make image)
set -uo pipefail

IMAGE="${IMAGE:-dominet:dev}"
PG_CT="${PG_CONTAINER:-dominet-dev-postgres-1}"
W="$(mktemp -d /tmp/dominet-backup-XXXXXX)"
SRC="bk_src_$$"; DST="bk_dst_$$"
RS_PORT="${OBS_RS_PORT:-18484}"
RS_IMAGE="${RS_IMAGE:-reduct/store:v1.21.5}"
fails=0
ok() { echo "  OK    $1"; }
ko() { echo "  ECHEC $1"; fails=$((fails + 1)); }
check() { if eval "$2" >/dev/null 2>&1; then ok "$1"; else ko "$1"; fi; }
psql_() { docker exec -i "$PG_CT" psql -qAt -U dominet -d "$1" -c "$2"; }

cleanup() {
  docker rm -f "bk-rs1-$$" "bk-rs2-$$" >/dev/null 2>&1
  docker exec "$PG_CT" psql -q -U dominet -d postgres -c "DROP DATABASE IF EXISTS $SRC WITH (FORCE)" -c "DROP DATABASE IF EXISTS $DST WITH (FORCE)" >/dev/null 2>&1
  docker run --rm -v "$W:/w" --entrypoint sh debian:trixie-slim -c "rm -rf /w/*" >/dev/null 2>&1
  rm -rf "$W"
}
trap cleanup EXIT

docker image inspect "$IMAGE" >/dev/null 2>&1 || { echo "image absente : make image" >&2; exit 2; }
docker exec "$PG_CT" psql -q -U dominet -d postgres -c "CREATE DATABASE $SRC" -c "CREATE DATABASE $DST" >/dev/null || { echo "stack de dev absente : make dev-up" >&2; exit 2; }
mkdir -p "$W/backups" "$W/rs1" "$W/rs2"; chmod 777 "$W/backups"

url() { echo "postgres://dominet:dominet-dev@127.0.0.1:15432/$1"; }
dom() { docker run --rm --user "$(id -u):$(id -g)" --network host -v "$W/backups:/backups" -v "$W/rs1:/reductstore-src:ro" -v "$W/rs2:/restore" \
  -e BACKUP_DIR=/backups -e REDUCTSTORE_BACKUP_SRC=/reductstore-src -e METRICS_ADDR=off -e DOMINET_MASTER_KEY="$KEY" "$@"; }
KEY="$(docker run --rm "$IMAGE" gen-master-key | tail -1)"

echo "== Base de donnees"
dom -e DATABASE_URL="$(url $SRC)" "$IMAGE" migrate >/dev/null
dom -e DATABASE_URL="$(url $SRC)" "$IMAGE" seed >/dev/null
psql_ "$SRC" "INSERT INTO location (name, description) VALUES ('Salon', 'Pièce « principale » — étage 1')" >/dev/null
psql_ "$SRC" "INSERT INTO device (friendly_name, device_type, state) VALUES ('Sonde', 'sensor', '{\"temperature\": 21.5, \"msg\": \"é — ü\"}')" >/dev/null

# ReductStore source : de vraies donnees ecrites par l'API
docker run -d --name "bk-rs1-$$" --user "$(id -u):$(id -g)" --network host -e RS_PORT=$RS_PORT -e RS_DATA_PATH=/data -e RS_API_TOKEN=bk-token -v "$W/rs1:/data" "$RS_IMAGE" >/dev/null
for _ in $(seq 1 60); do curl -fs "http://127.0.0.1:$RS_PORT/api/v1/alive" >/dev/null 2>&1 && break; sleep 0.5; done
RS="http://127.0.0.1:$RS_PORT/api/v1"
rc() { curl -H "Authorization: Bearer bk-token" "$@"; }   # ReductStore 1.21 exige un jeton
rc -fs -X POST "$RS/b/test" -d '{"quota_type":"FIFO","quota_size":1000000000}' >/dev/null
head -c 50000 /dev/urandom > "$W/rec1.bin"; head -c 12345 /dev/urandom > "$W/rec2.bin"
rc -fs -X POST "$RS/b/test/entree?ts=1000000" -H "Content-Type: application/octet-stream" -H "x-reduct-label-camera: cam1" --data-binary @"$W/rec1.bin" >/dev/null
rc -fs -X POST "$RS/b/test/entree?ts=2000000" -H "Content-Type: application/octet-stream" --data-binary @"$W/rec2.bin" >/dev/null
check "ReductStore source contient 2 enregistrements" "[ \"\$(rc -s $RS/b/test | python3 -c 'import sys,json; print(json.load(sys.stdin)[\"entries\"][0][\"record_count\"])')\" = 2 ]"
sleep 2   # laisse ReductStore ecrire ses blocs sur le disque

dom -e DATABASE_URL="$(url $SRC)" "$IMAGE" backup >"$W/backup.log" 2>&1
check "dominet backup reussit : $(grep -m1 '^Base' "$W/backup.log" | cut -c1-80)" "[ -s $W/backups/postgres/*.sql.gz ] && [ -s $W/backups/reductstore/*.tar.gz ]"
check "sommes de controle ecrites et valides (sha256sum -c)" "cd $W/backups/postgres && sha256sum -c *.sha256 && cd ../reductstore && sha256sum -c *.sha256"
dom -e DATABASE_URL="$(url $SRC)" "$IMAGE" backups >"$W/list.log" 2>&1
check "dominet backups liste les deux sauvegardes" "[ \$(grep -c -E 'postgres|reductstore' $W/list.log) -ge 2 ]"

# Restauration de la base dans une AUTRE base : refus sans --yes, puis comparaison table par table.
dom -e DATABASE_URL="$(url $DST)" "$IMAGE" restore --latest postgres >"$W/refuse.log" 2>&1
check "restauration refusee sans --yes" "grep -q -- '--yes' $W/refuse.log"
dom -e DATABASE_URL="$(url $DST)" "$IMAGE" restore --latest postgres --yes >"$W/restore.log" 2>&1
check "restauration de la base (pg_dump 17 -> serveur 16) : $(head -1 "$W/restore.log" | cut -c1-70)" "grep -q 'Base restauree' $W/restore.log"
fingerprint() {
  for t in $(psql_ "$1" "SELECT table_name FROM information_schema.tables WHERE table_schema='public' AND table_type='BASE TABLE' ORDER BY 1"); do
    echo "$t $(psql_ "$1" "SELECT count(*), md5(COALESCE(string_agg(x::text, E'\n' ORDER BY x::text), '')) FROM $t x")"
  done
}
fingerprint "$SRC" > "$W/fp-src.txt"; fingerprint "$DST" > "$W/fp-dst.txt"
check "memes tables et memes lignes dans la base restauree ($(wc -l < "$W/fp-src.txt") tables)" "[ -s $W/fp-src.txt ] && diff -q $W/fp-src.txt $W/fp-dst.txt"
check "accents et tirets intacts" "[ \"\$(psql_ $DST \"SELECT description FROM location WHERE name='Salon'\")\" = 'Pièce « principale » — étage 1' ]"

# Fichier abime
F="$(ls "$W"/backups/postgres/*.sql.gz | head -1)"
python3 - "$F" <<'PY'
import sys
p = sys.argv[1]; b = bytearray(open(p, "rb").read()); b[len(b) // 2] ^= 0xFF; open(p, "wb").write(b)
PY
psql_ "$DST" "DELETE FROM location" >/dev/null
dom -e DATABASE_URL="$(url $DST)" "$IMAGE" restore "/backups/postgres/$(basename "$F")" --yes >"$W/corrupt.log" 2>&1
check "fichier abime : restauration refusee (somme de controle)" "grep -q 'somme de controle' $W/corrupt.log"
check "la base n'a pas ete touchee par la restauration refusee" "[ \"\$(psql_ $DST 'SELECT count(*) FROM location')\" = 0 ]"

echo "== ReductStore"
docker rm -f "bk-rs1-$$" >/dev/null 2>&1
dom -e DATABASE_URL="$(url $SRC)" "$IMAGE" restore --latest reductstore --to /restore >"$W/rs-restore.log" 2>&1
check "archive du volume extraite : $(head -1 "$W/rs-restore.log" | cut -c1-70)" "grep -q 'extrait' $W/rs-restore.log"
dom -e DATABASE_URL="$(url $SRC)" "$IMAGE" restore --latest reductstore --to /restore >"$W/rs-refuse.log" 2>&1
check "dossier cible non vide : refus sans --force" "grep -q 'pas vide' $W/rs-refuse.log"
docker run -d --name "bk-rs2-$$" --user "$(id -u):$(id -g)" --network host -e RS_PORT=$RS_PORT -e RS_DATA_PATH=/data -e RS_API_TOKEN=bk-token -v "$W/rs2:/data" "$RS_IMAGE" >/dev/null
for _ in $(seq 1 60); do curl -fs "$RS/alive" >/dev/null 2>&1 && break; sleep 0.5; done
check "second ReductStore demarre sur le volume restaure" "curl -fs $RS/alive"
check "bucket et 2 enregistrements retrouves" "[ \"\$(rc -s $RS/b/test | python3 -c 'import sys,json; print(json.load(sys.stdin)[\"entries\"][0][\"record_count\"])')\" = 2 ]"
check "enregistrement 1 identique octet par octet" "rc -fs '$RS/b/test/entree?ts=1000000' -o $W/out1.bin && cmp $W/rec1.bin $W/out1.bin"
check "enregistrement 2 identique octet par octet" "rc -fs '$RS/b/test/entree?ts=2000000' -o $W/out2.bin && cmp $W/rec2.bin $W/out2.bin"

echo
if [ "$fails" -eq 0 ]; then echo "Sauvegardes : tout est vert"; else echo "Sauvegardes : $fails echec(s)"; echo "(journaux : $W)"; trap - EXIT; docker rm -f "bk-rs1-$$" "bk-rs2-$$" >/dev/null 2>&1; fi
exit "$fails"
