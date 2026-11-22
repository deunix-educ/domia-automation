# SPDX-License-Identifier: AGPL-3.0-or-later
# Cibles de developpement. Les cibles prod-* (prod-init, prod-deploy...) arrivent en R7.
SHELL := /bin/bash
.DEFAULT_GOAL := help

# Base des tests d'integration : la stack de dev (make dev-up). #[sqlx::test] cree une base isolee par test.
export DATABASE_URL ?= postgres://dominet:dominet-dev@127.0.0.1:15432/dominet
export SQLX_OFFLINE ?= true

.PHONY: fetch-model dev-run dev-discovery help build test fmt lint wasm front e2e e2e-image image-smoke deny audit ci ci-full hooks image dev-up dev-down dev-reset dev-ps dev-replay db-migrate sqlx-prepare sqlx-check preflight check-installer check-migrate prod-export prod-import prod-doctor prod-init prod-guard prod-deploy prod-up prod-migrate prod-seed prod-seed-energy prod-update prod-superuser prod-health prod-passwd prod-dump-context prod-backup prod-backups prod-report prod-video prod-mqtt-user prod-mqtt-del prod-mqtt-users prod-logs prod-ps prod-down i18n-new i18n-status certs export-ca demo-up demo-down docs docs-serve docs-deploy

help: ## Affiche cette aide
	@grep -E '^[a-zA-Z_-]+:.*## ' $(MAKEFILE_LIST) | awk -F':.*## ' '{printf "  %-10s %s\n", $$1, $$2}'

build: ## Compile le workspace
	cargo build --workspace

# Les tests `#[sqlx::test]` creent chacun une base : un `CREATE DATABASE` force un checkpoint, et avec ~20 tests en parallele (machine
# de 20 coeurs) la file depasse le delai de connexion de sqlx (`PoolTimedOut`). 8 fils suffisent ; `make test TEST_THREADS=20` pour voir.
TEST_THREADS ?= 8

test: ## Lance les tests (TEST_THREADS fils, 8 par defaut)
	RUST_TEST_THREADS=$(TEST_THREADS) cargo test --workspace

fmt: ## Formate le code
	cargo fmt --all

lint: ## fmt --check + clippy -D warnings (serveur et WASM)
	cargo fmt --all --check
	cargo clippy --workspace --all-targets -- -D warnings
	cargo clippy -p dominet-web --target wasm32-unknown-unknown --no-default-features --features hydrate -- -D warnings

wasm: ## Compile l'interface en WASM (feature hydrate)
	cargo build -p dominet-web --target wasm32-unknown-unknown --no-default-features --features hydrate

front: ## Build complet de l'interface (WASM + serveur) avec cargo-leptos -> target/site
	cargo leptos build

e2e: ## E2E navigateur sur target/debug/dominet (apres `make front`) ; base et Firefox jetables (tools/e2e.sh)
	tools/e2e.sh

e2e-image: ## E2E navigateur sur l'image Docker (IMAGE=dominet:dev par defaut) : le test qui compte pour livrer
	tools/e2e.sh --image $(or $(IMAGE),dominet:dev)

deny: ## Licences et advisories (cargo-deny)
	cargo deny check

audit: ## Vulnerabilites connues (cargo-audit)
	cargo audit

ci: lint test wasm deny audit sqlx-check check-installer ## Barriere avant commit (base de dev requise : make dev-up)

ci-full: ci image-smoke e2e-image ## ci + image Docker (build, fumee) + E2E navigateur sur l'image (avant un jalon, ou si le Dockerfile change)

hooks: ## Active le garde-fou pre-push (make ci sur une copie propre du commit pousse)
	git config core.hooksPath .githooks
	chmod +x .githooks/pre-push
	@echo "pre-push actif : voir .githooks/pre-push"

image: ## Construit l'image Docker (architecture de la machine ; le modele YOLO doit etre la : make fetch-model)
	@test -f models/yolov8n.onnx || $(MAKE) fetch-model
	docker build -t dominet:dev .

image-smoke: ## Construit l'image et verifie /healthz, /readyz, non-root, arret propre
	IMAGE=dominet:dev ci/image-smoke.sh

# Modele de detection YOLOv8n (12,9 Mo) : non commite (ADR 0050). Export officiel d'Ultralytics (licence AGPL-3.0) dans un conteneur
# jetable, avec la version et les options relevees dans les metadonnees du fichier de reference (ultralytics 8.4.82, opset 12, simplify).
# L'empreinte du fichier de reference est YOLO_SHA256 ; un autre export peut differer d'un octet : ce sont alors les tests de parite
# (`cargo test -p dominet-surveillance --test yolo_parity`) qui disent si les detections restent les memes.
ULTRALYTICS_VERSION := 8.4.82
YOLO_SHA256 := c74cfeb94d6fe7f4ee7b4c450932e97f3bc28e53461f9fd1f4b5baa3e60e20af

fetch-model: ## Exporte models/yolov8n.onnx (Ultralytics, conteneur jetable) et compare son empreinte
	mkdir -p models
	docker run --rm -v "$(CURDIR)/models:/out" -w /out python:3.13-slim sh -c "apt-get update -qq && apt-get install -y -qq --no-install-recommends libgl1 libglib2.0-0 >/dev/null && pip install --quiet --no-cache-dir --extra-index-url https://download.pytorch.org/whl/cpu ultralytics==$(ULTRALYTICS_VERSION) onnx onnxslim && yolo export model=yolov8n.pt format=onnx opset=12 simplify=True; status=\$$?; chown -R $(shell id -u):$(shell id -g) /out; exit \$$status"
	@got=$$(sha256sum models/yolov8n.onnx | cut -d' ' -f1); \
	if [ "$$got" = "$(YOLO_SHA256)" ]; then echo "Modele exporte : identique au fichier de reference (sha256 $$got)."; \
	else echo "Modele exporte (sha256 $$got)."; echo "Ce n'est pas une erreur : l'empreinte differe de la reference ($(YOLO_SHA256)), un export peut varier d'un octet."; echo "Pour verifier que les detections restent les memes : cargo test -p dominet-surveillance --test yolo_parity"; fi

DEV_COMPOSE := DB_PASSWORD=dominet-dev REDUCTSTORE_TOKEN=dominet-dev-token MQTT_PASSWORD=dominet-dev GRAFANA_PASSWORD=admin docker compose -f deploy/docker-compose.yml -f deploy/docker-compose.dev.yml
DEV_SERVICES := postgres reductstore mosquitto mediamtx

dev-up: ## Demarre la stack de dev jetable (PostgreSQL, ReductStore, Mosquitto, MediaMTX)
	mkdir -p deploy/recordings-dev
	DEV_UID=$$(id -u) DEV_GID=$$(id -g) $(DEV_COMPOSE) up -d --wait $(DEV_SERVICES)

dev-run: ## Lance DomiNet en dev sur http://127.0.0.1:8080 (stack de dev requise ; DISCOVERY=1 : role discovery ; RELEASE=1 : interface optimisee, page reactive en 1-2 s)
	tools/dev-run.sh $(if $(DISCOVERY),--discovery)

dev-discovery: ## Role discovery seul, second terminal a cote de make dev-run
	tools/dev-run.sh --discovery-only

dev-down: ## Arrete la stack de dev (volumes conserves)
	$(DEV_COMPOSE) --profile observability down

AGENT_MODES ?= gpu,cpu
agent-bench: ## Bench de l'assistant local (R12 A0) : LLAMA_DIR=<dossier de llama-server> MODELS_DIR=<dossier des .gguf> ; ecrit docs/bench/agent-<machine>-<date>.md
	@test -n "$(LLAMA_DIR)" || { echo "LLAMA_DIR=<dossier de llama-server> requis"; exit 1; }
	mkdir -p docs/bench target/agent-bench
	DUMP_TOOLS_DIR=target/agent-bench cargo test -p dominet-mcp --test dump -- --ignored
	python3 tools/agent-bench/bench.py --llama-dir "$(LLAMA_DIR)" --models-dir "$(or $(MODELS_DIR),$$HOME/IA/models)" \
	  --tools target/agent-bench/tools.json --modes "$(AGENT_MODES)" --out "docs/bench/agent-$$(hostname)-$$(date +%F).md"

agent-eval: ## Evaluation de l'assistant local (R12 A5, 51 demandes) : LLAMA_DIR=<dossier de llama-server> MODELS_DIR=<dossier des .gguf> AGENT_EVAL_MODELS=nom1,nom2 ; ecrit docs/bench/agent-eval-<machine>-<date>.md
	@test -n "$(LLAMA_DIR)" || { echo "LLAMA_DIR=<dossier de llama-server> requis"; exit 1; }
	mkdir -p docs/bench
	python3 tools/agent-bench/eval.py --llama-dir "$(LLAMA_DIR)" --models-dir "$(or $(MODELS_DIR),$$HOME/IA/models)" \
	  $(if $(AGENT_EVAL_MODELS),--models "$(AGENT_EVAL_MODELS)") --out "docs/bench/agent-eval-$$(hostname)-$$(date +%F).md"

agent-install: ## Installe l'assistant local sur la machine d'inference (R12 A6) : LLAMA_DIR=<dossier de llama-server> [MODELS_DIR=...] [AGENT_PORT=8081] [AGENT_HOST=0.0.0.0] [DRY=1]
	@test -n "$(LLAMA_DIR)" || { echo "LLAMA_DIR=<dossier de llama-server> requis"; exit 1; }
	tools/agent-install.sh --llama-dir "$(LLAMA_DIR)" $(if $(MODELS_DIR),--models-dir "$(MODELS_DIR)") \
	  $(if $(AGENT_PORT),--port "$(AGENT_PORT)") $(if $(AGENT_HOST),--host "$(AGENT_HOST)") $(if $(DRY),--dry-run)

dev-reset: ## Arrete la stack de dev et supprime ses volumes
	$(DEV_COMPOSE) --profile observability down -v

dev-ps: ## Etat de la stack de dev
	$(DEV_COMPOSE) ps

dev-replay: ## Rejoue les fixtures Z2M vers le Mosquitto de dev
	tools/replay-z2m.sh

db-migrate: ## Applique les migrations sur la base de dev (comme en production : `dominet migrate`, qui accepte les anciens checksums connus)
	cargo run -q -p dominet -- migrate

sqlx-prepare: db-migrate ## Regenere .sqlx/ (metadonnees des requetes verifiees a la compilation)
	SQLX_OFFLINE=false cargo sqlx prepare --workspace -- --all-targets

sqlx-check: db-migrate ## Verifie que .sqlx/ est a jour
	SQLX_OFFLINE=false cargo sqlx prepare --workspace --check -- --all-targets

# ---------------------------------------------------------------------------------------------------------------------------------
# Exploitation (R7 lot 6) : tout passe par tools/prod.sh. Ne JAMAIS lancer ces cibles sur une machine que l'on ne veut pas deployer.
# Variables : ENV_FILE (.env), PROJECT (dominet), CERTS_DIR (certs). Installation vierge : prod-init, relire .env, prod-deploy.
# ---------------------------------------------------------------------------------------------------------------------------------
check-installer: ## Teste l'assistant d'installation (install.sh --dry-run avec des programmes factices : rien n'est installe)
	tools/check-installer.sh

preflight: ## Verifie la machine avant installation (lecture seule : ports, Docker, disque, dongle, existant)
	tools/preflight.sh

prod-init: ## Cree .env (secrets generes, reseau detecte) a partir de .env.example.prod
	@tools/prod.sh init

prod-export: ## Archive de migration (base, ReductStore, secrets chiffres par phrase de passe) : OUT=dossier ; voir docs/migration.md
	@tools/migrate-pack.sh export $(if $(OUT),--out $(OUT),)

prod-import: ## Importe une archive sur CETTE machine : FILE=archive.tar [DRY=1] [YES=1] ; voir docs/migration.md
	@[ -n "$(FILE)" ] || { echo "FILE=archive.tar requis"; exit 2; }
	@tools/migrate-pack.sh import "$(FILE)" $(if $(DRY),--dry-run,) $(if $(YES),--yes,)

check-migrate: ## Repetition complete export/import sur deux piles jetables (arreter la pile de dev avant : make dev-down)
	tools/check-migrate.sh dominet:dev

prod-doctor: ## Diagnostic en lecture seule : machine, services, certificat, application (dit quoi corriger)
	@tools/prod.sh doctor

prod-guard: ## Refuse de continuer si .env est absent, vide ou garde des valeurs par defaut
	@tools/prod.sh guard

certs: ## Certificat TLS (mkcert si installe, sinon openssl auto-signe) dans certs/
	@tools/prod.sh certs

export-ca: ## Exporte l'autorite locale vers certs/dominet-ca.crt (servie sur /dominet-ca.crt)
	@tools/prod.sh export-ca

prod-deploy: ## Installation : certificats, construction, demarrage, administrateur, donnees initiales, verification
	@tools/prod.sh deploy

prod-up: ## Construit et (re)demarre la pile de production
	@tools/prod.sh up

prod-migrate: ## Applique les migrations (deja faites au demarrage par le service migrate)
	@tools/prod.sh migrate

prod-seed: ## Donnees initiales (dominet seed) et regles d'alerte (sync-rules)
	@tools/prod.sh seed

prod-seed-energy: ## Suit les cles du bilan et du tableau Energie (HCHP, HCHC, PAPP, qdy, pac, energy_a/b, power_ab) : DRY=1 pour simuler
	@tools/prod.sh seed-energy

prod-superuser: ## Cree le compte administrateur s'il n'y en a pas (DOMINET_ADMIN_USER / DOMINET_ADMIN_PASSWORD sinon saisie)
	@tools/prod.sh superuser

prod-update: ## Mise a jour : sauvegarde, git pull, reconstruction, verification (NOBACKUP=1, NOPULL=1)
	@tools/prod.sh update

prod-health: ## Verifie nginx -> application et l'absence de service en boucle de redemarrage
	@tools/prod.sh health

prod-passwd: ## Change un mot de passe (saisie interactive) : make prod-passwd U=nom
	@tools/prod.sh passwd

prod-dump-context: ## Instantane YAML de l'installation (appareils, etats, schemas)
	@tools/prod.sh dump-context

prod-backups: ## Liste les sauvegardes du volume backups-data
	tools/prod.sh backups

prod-mqtt-user: ## Compte MQTT d'un client du LAN : make prod-mqtt-user NAME=linky [TOPIC=linky/#] [ACCESS=readwrite]
	NAME='$(NAME)' TOPIC='$(TOPIC)' ACCESS='$(ACCESS)' tools/prod.sh mqtt-user

prod-mqtt-del: ## Supprime un compte MQTT : make prod-mqtt-del NAME=linky
	NAME='$(NAME)' tools/prod.sh mqtt-del

prod-mqtt-users: ## Liste les comptes MQTT et leurs droits
	tools/prod.sh mqtt-users

prod-backup: ## Sauvegarde immediate (base + ReductStore) dans le volume backups-data
	@tools/prod.sh backup

prod-report: ## Bilan de surveillance en LECTURE SEULE (services, ressources, journaux, automations, alertes, sauvegardes ; HOURS=12)
	@tools/prod.sh report

prod-video: ## Active le decodage video materiel (VAAPI) dans .env si /dev/dri/renderD128 existe ; puis make prod-up
	@tools/prod.sh video

prod-logs: ## Journaux de la pile (S=service ; N=100 : les N dernieres lignes ; SINCE=3h : depuis 3 h ; sans suivre)
	@tools/prod.sh logs

prod-ps: ## Etat de la pile de production
	@tools/prod.sh ps

prod-down: ## Arrete la pile de production (volumes conserves)
	@tools/prod.sh down

DEMO_COMPOSE = docker compose -p dominet-demo --env-file .env.demo -f deploy/docker-compose.yml -f deploy/docker-compose.demo.yml

demo-up: ## Demo (HTTP, camera factice) : copier .env.example.demo en .env.demo, puis make demo-up
	@test -f .env.demo || { echo "  .env.demo manquant : cp .env.example.demo .env.demo"; exit 1; }
	$(DEMO_COMPOSE) build dominet
	$(DEMO_COMPOSE) up -d --wait --wait-timeout 300
	$(DEMO_COMPOSE) exec -T dominet dominet seed

demo-down: ## Arrete la demo (volumes conserves ; ajouter -v a la main pour les supprimer)
	$(DEMO_COMPOSE) down

i18n-new: ## Cree une langue de l'interface depuis l'anglais (LANGUE=es ; FROM=fr pour partir du francais)
	@test -n "$(LANGUE)" || (echo "usage : make i18n-new LANGUE=es [FROM=fr]" >&2; exit 2)
	@python3 tools/i18n.py new "$(LANGUE)" $(if $(FROM),--from $(FROM),)

i18n-status: ## Etat des traductions : messages a traduire, cles ou variables en ecart (LIST=1 pour les lister)
	@python3 tools/i18n.py status $(if $(LIST),--list,)

DOCS_VENV ?= $(HOME)/.venvs/dominet-docs
docs: ## Documentation MkDocs FR/EN -> target/docs-site (venv cree au besoin ; servie sur /docs/ par l'application)
	@test -x $(DOCS_VENV)/bin/mkdocs || (python3 -m venv $(DOCS_VENV) && $(DOCS_VENV)/bin/pip install -q -r requirements-docs.txt)
	NO_MKDOCS_2_WARNING=1 $(DOCS_VENV)/bin/mkdocs build --strict -q

docs-deploy: ## Publie le site sur la branche gh-pages (A LA MAIN, apres le feu vert ; voir docs/releasing.md)
	NO_MKDOCS_2_WARNING=1 $(DOCS_VENV)/bin/mkdocs gh-deploy --force

docs-serve: ## Apercu local de la documentation (http://127.0.0.1:8001)
	@test -x $(DOCS_VENV)/bin/mkdocs || (python3 -m venv $(DOCS_VENV) && $(DOCS_VENV)/bin/pip install -q -r requirements-docs.txt)
	NO_MKDOCS_2_WARNING=1 $(DOCS_VENV)/bin/mkdocs serve --dev-addr 127.0.0.1:8001
