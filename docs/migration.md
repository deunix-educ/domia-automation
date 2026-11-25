# Moving to another machine

One archive carries everything: the database, the video and history (ReductStore), and, encrypted with a passphrase, the keys and files you would otherwise lose. The old machine is only **read** (plus one normal backup); you decide when to switch over.

## What moves, and what does not

| Moves | Where it comes from | Notes |
|---|---|---|
| Accounts, devices, rules, automations, scenes, cameras, settings | database backup | same passwords: the hashes are restored |
| History and archived video | ReductStore backup | `--skip-reductstore` leaves it out (large) |
| `.env` and `DOMINET_MASTER_KEY` | encrypted part | without the key, stored camera passwords are unreadable |
| The mkcert authority (key included) and the certificate | encrypted part | **devices keep trusting the new machine**, nothing to reinstall |
| Zigbee2MQTT folder | encrypted part | the pairing is kept, no device to pair again |
| MQTT accounts of your LAN clients | encrypted part | |
| VPN (wg-easy 15: administrator, settings, server keys, devices) | encrypted part | an archive made with the wg-easy 14 format is reported and not restored (the VPN starts empty) |
| Grafana data | encrypted part | only with `--with-grafana` |

Not moved: live recordings in progress (the `recordings` volume holds fragments not yet archived), container logs, and the Docker images themselves (the new machine pulls or builds them).

## Steps

**1. On the old machine** (stack running):

```bash
make prod-export OUT=/path/to/usb      # asks a passphrase twice
```

It runs one normal backup, copies the newest backups, gathers the files, encrypts the keys (AES-256, PBKDF2 600 000 rounds) and writes `dominet-migration-<date>.tar` with its `.sha256`. Keep the passphrase **apart** from the archive.

**2. On the new machine**: install the prerequisites (Docker, Compose, `git`, `make`, `openssl`, `curl`, `mkcert` for the authority), `git clone` the repository **at a version at least as recent as the old one**, copy the archive, then try without changing anything:

```bash
make prod-import FILE=/path/to/dominet-migration-….tar DRY=1
```

It checks the archive and the passphrase, shows what it contains, compares the architecture, the PostgreSQL major and the schema version, and lists what it would do.

**3. Import for real**:

```bash
make prod-import FILE=/path/to/dominet-migration-….tar
```

The importer writes the configuration (adapting the address, the Zigbee dongle port and hardware video to this machine), restores the authority and certificates (issuing a new certificate from the same authority if the address changed), starts the stack, restores the MQTT accounts, the database then ReductStore, applies the migrations the old machine did not have, restarts and checks. It **refuses to overwrite an existing installation** unless you add `YES=1` (the old `.env` is then kept as `.env.before-import-<date>`).

**Rehearsal on a second machine while the old one keeps running:** use `--no-zigbee` (`tools/migrate-pack.sh import FILE --no-zigbee`). The archive carries the **same** Zigbee network as the old machine (same key, same identifiers): two coordinators with that identity on one LAN confuse the devices. Never plug the production dongle and a test dongle running that network at the same time. If the machine already holds a database volume of an earlier installation (even stopped), the importer refuses without `--yes`; with `--yes` it removes that volume and the project's containers first (their data is erased), because the old database keeps the old password.

**4. Switch over.** Point the name your devices use (`LAN_DOMAIN`, for example `dominet.local`) at the new machine: give it the old host name, or change the entry on your router. If you use a Zigbee dongle, move it, set `ZIGBEE_DEVICE` in `.env` if the importer did not find it, and `make prod-up`. Check with `make prod-doctor`.

## Between architectures and versions

- amd64 to arm64 (and back) is supported by the data formats: the database is a logical dump, ReductStore data files do not depend on the processor. Only the **image** differs, and the importer uses the one of the machine it runs on. This is untested on real arm64 hardware (see [Supported machines](machines.md)).
- The PostgreSQL major version must match (the importer refuses otherwise). A newer code than the archive is fine: missing migrations are applied. An **older** code than the archive is refused: update the repository first.

## Rehearsal

`make check-migrate` plays the whole procedure on two throw-away stacks on one machine: administrator, data, a ReductStore record, an MQTT account and Zigbee files are exported, the source is destroyed, a target imports, and everything is compared (accounts and password hashes, rows, record, keys, authority, certificate signature, accounts, files). It also checks the refusals (wrong passphrase, damaged archive, existing installation, dry run). Stop the development stack first (`make dev-down`), restart it after (`make dev-up`).

Before relying on a move, rehearse it with your own data: **old machine → new machine, in dry-run mode, then for real on the new one while the old one keeps running**; only then switch the name over.

## Limits

- The passphrase protects the keys against theft of the archive; it does not authenticate the archive against someone who can modify it. Keep it in your own custody.
- The archive holds your data: video and history in clear. Encrypt the place where you store it.
- A database restore replaces the target's data; there is no merge.
