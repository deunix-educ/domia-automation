# SPDX-License-Identifier: AGPL-3.0-or-later
# Development targets. The prod-* targets (prod-init, prod-deploy...) arrive in R7.
SHELL := /bin/bash
.DEFAULT_GOAL := help

# Base of the integration tests: the dev stack (make dev-up). #[sqlx::test] creates one isolated database per test.
export DATABASE_URL ?= postgres://dominet:dominet-dev@127.0.0.1:15432/dominet
export SQLX_OFFLINE ?= true

.PHONY: fetch-model dev-run dev-discovery help build test fmt lint wasm front e2e e2e-image image-smoke deny audit ci ci-full hooks image dev-up dev-down dev-reset dev-ps dev-replay db-migrate sqlx-prepare sqlx-check preflight check-installer check-migrate prod-export prod-import prod-doctor prod-init prod-guard prod-deploy prod-up prod-migrate prod-seed prod-seed-energy prod-update prod-superuser prod-health prod-passwd prod-dump-context prod-backup prod-backups prod-report prod-video prod-mqtt-user prod-mqtt-del prod-mqtt-users prod-logs prod-ps prod-down i18n-new i18n-status certs export-ca android-apk demo-up demo-down docs docs-serve docs-deploy

help: ## Show this help
	@grep -E '^[a-zA-Z_-]+:.*## ' $(MAKEFILE_LIST) | awk -F':.*## ' '{printf "  %-10s %s\n", $$1, $$2}'

build: ## Build the workspace
	cargo build --workspace

# The `#[sqlx::test]` tests each create a database: a `CREATE DATABASE` forces a checkpoint, and with ~20 tests in parallel (20-core
# machine) the queue exceeds sqlx's connection timeout (`PoolTimedOut`). 8 threads are enough; `make test TEST_THREADS=20` to see.
TEST_THREADS ?= 8

test: ## Run the tests (TEST_THREADS threads, 8 by default)
	RUST_TEST_THREADS=$(TEST_THREADS) cargo test --workspace

fmt: ## Format the code
	cargo fmt --all

lint: ## fmt --check + clippy -D warnings (server and WASM)
	cargo fmt --all --check
	cargo clippy --workspace --all-targets -- -D warnings
	cargo clippy -p dominet-web --target wasm32-unknown-unknown --no-default-features --features hydrate -- -D warnings

wasm: ## Build the interface as WASM (hydrate feature)
	cargo build -p dominet-web --target wasm32-unknown-unknown --no-default-features --features hydrate

front: ## Full interface build (WASM + server) with cargo-leptos -> target/site
	cargo leptos build

e2e: ## Browser E2E on target/debug/dominet (after `make front`); throw-away database and Firefox (tools/e2e.sh)
	tools/e2e.sh

e2e-image: ## Browser E2E on the Docker image (IMAGE=dominet:dev by default): the test that counts before delivering
	tools/e2e.sh --image $(or $(IMAGE),dominet:dev)

deny: ## Licences and advisories (cargo-deny)
	cargo deny check

audit: ## Known vulnerabilities (cargo-audit)
	cargo audit

ci: lint test wasm deny audit sqlx-check check-installer ## Gate before commit (dev database required: make dev-up)

ci-full: ci image-smoke e2e-image ## ci + Docker image (build, smoke test) + browser E2E on the image (before a milestone, or if the Dockerfile changes)

hooks: ## Enable the pre-push safeguard (make ci on a clean copy of the pushed commit)
	git config core.hooksPath .githooks
	chmod +x .githooks/pre-push
	@echo "pre-push actif : voir .githooks/pre-push"

image: ## Build the Docker image (the machine's architecture; the YOLO model must be there: make fetch-model)
	@test -f models/yolov8n.onnx || $(MAKE) fetch-model
	docker build -t dominet:dev .

image-smoke: ## Build the image and check /healthz, /readyz, non-root, clean shutdown
	IMAGE=dominet:dev ci/image-smoke.sh

# YOLOv8n detection model (12.9 MB): not committed (ADR 0050). Official Ultralytics export (AGPL-3.0 licence) in a throw-away
# container, with the version and options taken from the reference file's metadata (ultralytics 8.4.82, opset 12, simplify).
# The reference file's fingerprint is YOLO_SHA256; another export may differ by a byte: the parity tests
# (`cargo test -p dominet-surveillance --test yolo_parity`) then tell whether the detections stay the same.
ULTRALYTICS_VERSION := 8.4.82
YOLO_SHA256 := c74cfeb94d6fe7f4ee7b4c450932e97f3bc28e53461f9fd1f4b5baa3e60e20af

fetch-model: ## Export models/yolov8n.onnx (Ultralytics, throw-away container) and compare its fingerprint
	mkdir -p models
	docker run --rm -v "$(CURDIR)/models:/out" -w /out python:3.13-slim sh -c "apt-get update -qq && apt-get install -y -qq --no-install-recommends libgl1 libglib2.0-0 >/dev/null && pip install --quiet --no-cache-dir --extra-index-url https://download.pytorch.org/whl/cpu ultralytics==$(ULTRALYTICS_VERSION) onnx onnxslim && yolo export model=yolov8n.pt format=onnx opset=12 simplify=True; status=\$$?; chown -R $(shell id -u):$(shell id -g) /out; exit \$$status"
	@got=$$(sha256sum models/yolov8n.onnx | cut -d' ' -f1); \
	if [ "$$got" = "$(YOLO_SHA256)" ]; then echo "Modele exporte : identique au fichier de reference (sha256 $$got)."; \
	else echo "Modele exporte (sha256 $$got)."; echo "Ce n'est pas une erreur : l'empreinte differe de la reference ($(YOLO_SHA256)), un export peut varier d'un octet."; echo "Pour verifier que les detections restent les memes : cargo test -p dominet-surveillance --test yolo_parity"; fi

DEV_COMPOSE := DB_PASSWORD=dominet-dev REDUCTSTORE_TOKEN=dominet-dev-token MQTT_PASSWORD=dominet-dev GRAFANA_PASSWORD=admin docker compose -f deploy/docker-compose.yml -f deploy/docker-compose.dev.yml
DEV_SERVICES := postgres reductstore mosquitto mediamtx

dev-up: ## Start the throw-away dev stack (PostgreSQL, ReductStore, Mosquitto, MediaMTX)
	mkdir -p deploy/recordings-dev
	DEV_UID=$$(id -u) DEV_GID=$$(id -g) $(DEV_COMPOSE) up -d --wait $(DEV_SERVICES)

dev-run: ## Run DomiNet in dev on http://127.0.0.1:8080 (dev stack required; DISCOVERY=1: discovery role; RELEASE=1: optimised interface, page reacts in 1-2 s)
	tools/dev-run.sh $(if $(DISCOVERY),--discovery)

dev-discovery: ## Discovery role alone, second terminal next to make dev-run
	tools/dev-run.sh --discovery-only

dev-down: ## Stop the dev stack (volumes kept)
	$(DEV_COMPOSE) --profile observability down

AGENT_MODES ?= gpu,cpu
agent-bench: ## Benchmark of the local assistant (R12 A0): LLAMA_DIR=<llama-server folder> MODELS_DIR=<.gguf folder>; writes docs/bench/agent-<machine>-<date>.md
	@test -n "$(LLAMA_DIR)" || { echo "LLAMA_DIR=<dossier de llama-server> requis"; exit 1; }
	mkdir -p docs/bench target/agent-bench
	DUMP_TOOLS_DIR=target/agent-bench cargo test -p dominet-mcp --test dump -- --ignored
	python3 tools/agent-bench/bench.py --llama-dir "$(LLAMA_DIR)" --models-dir "$(or $(MODELS_DIR),$$HOME/IA/models)" \
	  --tools target/agent-bench/tools.json --modes "$(AGENT_MODES)" --out "docs/bench/agent-$$(hostname)-$$(date +%F).md"

agent-eval: ## Evaluation of the local assistant (R12 A5, 51 requests): LLAMA_DIR=<llama-server folder> MODELS_DIR=<.gguf folder> AGENT_EVAL_MODELS=name1,name2; writes docs/bench/agent-eval-<machine>-<date>.md
	@test -n "$(LLAMA_DIR)" || { echo "LLAMA_DIR=<dossier de llama-server> requis"; exit 1; }
	mkdir -p docs/bench
	python3 tools/agent-bench/eval.py --llama-dir "$(LLAMA_DIR)" --models-dir "$(or $(MODELS_DIR),$$HOME/IA/models)" \
	  $(if $(AGENT_EVAL_MODELS),--models "$(AGENT_EVAL_MODELS)") --out "docs/bench/agent-eval-$$(hostname)-$$(date +%F).md"

agent-install: ## Install the local assistant on the inference machine (R12 A6): LLAMA_DIR=<llama-server folder> [MODELS_DIR=...] [AGENT_PORT=8081] [AGENT_HOST=0.0.0.0] [DRY=1]
	@test -n "$(LLAMA_DIR)" || { echo "LLAMA_DIR=<dossier de llama-server> requis"; exit 1; }
	tools/agent-install.sh --llama-dir "$(LLAMA_DIR)" $(if $(MODELS_DIR),--models-dir "$(MODELS_DIR)") \
	  $(if $(AGENT_PORT),--port "$(AGENT_PORT)") $(if $(AGENT_HOST),--host "$(AGENT_HOST)") $(if $(DRY),--dry-run)

dev-reset: ## Stop the dev stack and delete its volumes
	$(DEV_COMPOSE) --profile observability down -v

dev-ps: ## State of the dev stack
	$(DEV_COMPOSE) ps

dev-replay: ## Replay the Z2M fixtures to the dev Mosquitto
	tools/replay-z2m.sh

db-migrate: ## Apply the migrations to the dev database (as in production: `dominet migrate`, which accepts the known old checksums)
	cargo run -q -p dominet -- migrate

sqlx-prepare: db-migrate ## Regenerate .sqlx/ (metadata of the queries checked at compile time)
	SQLX_OFFLINE=false cargo sqlx prepare --workspace -- --all-targets

sqlx-check: db-migrate ## Check that .sqlx/ is up to date
	SQLX_OFFLINE=false cargo sqlx prepare --workspace --check -- --all-targets

# ---------------------------------------------------------------------------------------------------------------------------------
# Operation (R7 lot 6): everything goes through tools/prod.sh. NEVER run these targets on a machine you do not want to deploy.
# Variables: ENV_FILE (.env), PROJECT (dominet), CERTS_DIR (certs). Fresh installation: prod-init, review .env, prod-deploy.
# ---------------------------------------------------------------------------------------------------------------------------------
check-installer: ## Test the installation assistant (install.sh --dry-run with fake programs: nothing is installed)
	tools/check-installer.sh

preflight: ## Check the machine before installation (read-only: ports, Docker, disk, dongle, existing setup)
	tools/preflight.sh

prod-init: ## Create .env (generated secrets, detected network) from .env.example.prod
	@tools/prod.sh init

prod-export: ## Migration archive (database, ReductStore, secrets encrypted by passphrase): OUT=folder; see docs/migration.md
	@tools/migrate-pack.sh export $(if $(OUT),--out $(OUT),)

prod-import: ## Import an archive on THIS machine: FILE=archive.tar [DRY=1] [YES=1]; see docs/migration.md
	@[ -n "$(FILE)" ] || { echo "FILE=archive.tar requis"; exit 2; }
	@tools/migrate-pack.sh import "$(FILE)" $(if $(DRY),--dry-run,) $(if $(YES),--yes,)

check-migrate: ## Full export/import rehearsal on two throw-away stacks (stop the dev stack first: make dev-down)
	tools/check-migrate.sh dominet:dev

prod-doctor: ## Read-only diagnostic: machine, services, certificate, application (says what to fix)
	@tools/prod.sh doctor

prod-guard: ## Refuse to continue if .env is missing, empty or keeps default values
	@tools/prod.sh guard

certs: ## TLS certificate (mkcert if installed, otherwise self-signed openssl) in certs/
	@tools/prod.sh certs

android-apk: ## Build the signed Android APK (DomoComm) into dist/ (needs the SDK and the signing key, see docs/android.md)
	@tools/build-android.sh

export-ca: ## Export the local authority to certs/dominet-ca.crt (served on /dominet-ca.crt)
	@tools/prod.sh export-ca

prod-deploy: ## Installation: certificates, build, start, administrator, initial data, verification
	@tools/prod.sh deploy

prod-up: ## Build and (re)start the production stack
	@tools/prod.sh up

prod-migrate: ## Apply the migrations (already done at start-up by the migrate service)
	@tools/prod.sh migrate

prod-seed: ## Initial data (dominet seed) and alert rules (sync-rules)
	@tools/prod.sh seed

prod-seed-energy: ## Track the keys of the summary and of the Energy dashboard (HCHP, HCHC, PAPP, qdy, pac, energy_a/b, power_ab): DRY=1 to simulate
	@tools/prod.sh seed-energy

prod-superuser: ## Create the administrator account if there is none (DOMINET_ADMIN_USER / DOMINET_ADMIN_PASSWORD, otherwise prompted)
	@tools/prod.sh superuser

prod-update: ## Update: backup, git pull, rebuild, verification (NOBACKUP=1, NOPULL=1)
	@tools/prod.sh update

prod-health: ## Check nginx -> application and the absence of a service in a restart loop
	@tools/prod.sh health

prod-passwd: ## Change a password (interactive prompt): make prod-passwd U=name
	@tools/prod.sh passwd

prod-dump-context: ## YAML snapshot of the installation (devices, states, schemas)
	@tools/prod.sh dump-context

prod-backups: ## List the backups of the backups-data volume
	tools/prod.sh backups

prod-mqtt-user: ## MQTT account of a LAN client: make prod-mqtt-user NAME=linky [TOPIC=linky/#] [ACCESS=readwrite]
	NAME='$(NAME)' TOPIC='$(TOPIC)' ACCESS='$(ACCESS)' tools/prod.sh mqtt-user

prod-mqtt-del: ## Delete an MQTT account: make prod-mqtt-del NAME=linky
	NAME='$(NAME)' tools/prod.sh mqtt-del

prod-mqtt-users: ## List the MQTT accounts and their rights
	tools/prod.sh mqtt-users

prod-backup: ## Immediate backup (database + ReductStore) into the backups-data volume
	@tools/prod.sh backup

prod-report: ## READ-ONLY monitoring summary (services, resources, logs, automations, alerts, backups; HOURS=12)
	@tools/prod.sh report

prod-video: ## Enable hardware video decoding (VAAPI) in .env if /dev/dri/renderD128 exists; then make prod-up
	@tools/prod.sh video

prod-logs: ## Logs of the stack (S=service; N=100: the last N lines; SINCE=3h: since 3 h; without following)
	@tools/prod.sh logs

prod-ps: ## State of the production stack
	@tools/prod.sh ps

prod-down: ## Stop the production stack (volumes kept)
	@tools/prod.sh down

DEMO_COMPOSE = docker compose -p dominet-demo --env-file .env.demo -f deploy/docker-compose.yml -f deploy/docker-compose.demo.yml

demo-up: ## Demo (HTTP, fake camera): copy .env.example.demo to .env.demo, then make demo-up
	@test -f .env.demo || { echo "  .env.demo manquant : cp .env.example.demo .env.demo"; exit 1; }
	$(DEMO_COMPOSE) build dominet
	$(DEMO_COMPOSE) up -d --wait --wait-timeout 300
	$(DEMO_COMPOSE) exec -T dominet dominet seed

demo-down: ## Stop the demo (volumes kept; add -v by hand to delete them)
	$(DEMO_COMPOSE) down

i18n-new: ## Create an interface language from English (LANGUE=es; FROM=fr to start from French)
	@test -n "$(LANGUE)" || (echo "usage : make i18n-new LANGUE=es [FROM=fr]" >&2; exit 2)
	@python3 tools/i18n.py new "$(LANGUE)" $(if $(FROM),--from $(FROM),)

i18n-status: ## State of the translations: messages to translate, keys or variables out of line (LIST=1 to list them)
	@python3 tools/i18n.py status $(if $(LIST),--list,)

DOCS_VENV ?= $(HOME)/.venvs/dominet-docs
docs: ## MkDocs documentation FR/EN -> target/docs-site (venv created if needed; served on /docs/ by the application)
	@test -x $(DOCS_VENV)/bin/mkdocs || (python3 -m venv $(DOCS_VENV) && $(DOCS_VENV)/bin/pip install -q -r requirements-docs.txt)
	NO_MKDOCS_2_WARNING=1 $(DOCS_VENV)/bin/mkdocs build --strict -q

docs-deploy: ## Publish the site to the gh-pages branch (BY HAND, after the go-ahead; see docs/releasing.md)
	NO_MKDOCS_2_WARNING=1 $(DOCS_VENV)/bin/mkdocs gh-deploy --force

docs-serve: ## Local preview of the documentation (http://127.0.0.1:8001)
	@test -x $(DOCS_VENV)/bin/mkdocs || (python3 -m venv $(DOCS_VENV) && $(DOCS_VENV)/bin/pip install -q -r requirements-docs.txt)
	NO_MKDOCS_2_WARNING=1 $(DOCS_VENV)/bin/mkdocs serve --dev-addr 127.0.0.1:8001
