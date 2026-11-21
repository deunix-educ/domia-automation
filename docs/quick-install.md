# Quick install

The guided installer checks your machine, asks four questions and deploys the application. It takes a few minutes of your time; the first start may take longer while images are built or downloaded.

## What you need

- A machine that stays on at home: a mini PC, a small server or a Raspberry Pi 5 (see [Supported machines](machines.md) and [Measurements](measures.md) for what each holds). Linux, with **Docker** and the **Docker Compose** plugin (version 2.24 or newer), plus `git`, `make`, `openssl` and `curl`.
- At least 2 GB of memory (4 GB or more recommended) and 10 GB of free disk space (video recordings need much more).
- Your user must be able to run `docker` (member of the `docker` group).

## Install

```bash
git clone https://github.com/deunix-educ/domia-automation.git
cd domia-automation
./install.sh
```

The installer goes through four steps and stops with a clear message if something blocks:

1. **Checks the machine**: system, Docker and Compose versions, memory, disk space, free ports (80, 443, 1883), architecture.
2. **Asks a few questions**: the address the devices will use (default `<machine name>.local`), the administrator account name and password (hidden, at least 10 characters), an e-mail address for alerts (optional). It detects a Zigbee dongle and hardware video decoding by itself.
3. **Writes the configuration** (`.env`, readable by you only): all passwords and keys are generated for you and never displayed.
4. **Deploys**: certificates, build or download, start, administrator account, first data, final check.

## Try it first

```bash
./install.sh --dry-run
```

runs the checks and the questions and **changes nothing**. `./install.sh --help` lists the options (`--yes` for unattended use, `--domain`, `--admin`, `--email`, `--no-zigbee`, `--reuse`, `--lang fr|en`).

## After the installation

1. Open `https://<address>` and sign in with the administrator account.
2. On each phone and computer, install the certificate once by opening `http://<address>/dominet-ca.crt` (the application uses a certificate signed by your own authority; the page explains why browsers ask).
3. **Copy `DOMINET_MASTER_KEY` from `.env` to a safe place away from the machine.** Without it, a restored backup loses the encrypted secrets (camera passwords).
4. Add your devices and cameras; the rest of this documentation describes each page.

## Images

The installer first tries a ready-made image (amd64 and arm64) from the project's registry, which saves 15 to 40 minutes of compilation; if it is not available (offline, before publication, a fork) it builds the image on the machine. `DOMINET_PULL=0` in `.env` forces the build; `DOMINET_IMAGE_VERSION` pins a release. See [ADR 0062](adr/0062-published-images-with-local-build-fallback.md).

## If something goes wrong

- The installer names the blocking point and how to fix it (for example "the port 443 is already used": stop what uses it).
- It never overwrites an existing `.env`. To redo a failed deployment with the same configuration: `./install.sh --reuse`.
- To follow the application's messages: `make prod-logs S=dominet N=100`.
- The diagnosis, read-only, which says what to fix: `make prod-doctor` (see [Troubleshooting](troubleshooting.md)).
- Before installing: `make preflight`.

For the manual procedure, every option and the update routine, see [Production deployment](deployment.md).
