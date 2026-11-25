# Roadmap

Where the project stands and what is open. Decisions are in `docs/adr/`; known limits are in `docs/to-improve.md`; history is in `CHANGELOG.md`.

## State (release 1.0.2, 2026-11-23)

- Public repository, releases 1.0.0 to 1.0.2, multi-architecture image (amd64 and arm64) on GHCR, documentation site on GitHub Pages.
- Installation for non-experts: `install.sh`, `make prod-doctor`, migration of an existing installation (`tools/migrate-pack.sh`).
- Machines: the reference mini PC (12 cameras in 1080p with hardware decoding) and a Raspberry Pi 5 8 GB with NVMe (installs and runs; 6 cameras in 1080p at 40 % of the machine, software decoding). See `docs/machines.md` and `docs/measures.md`.
- No more than 6 cameras were measured on the Raspberry Pi 5, on purpose: the figure above that is an extrapolation and is documented as such.

## Open

Maintenance
- A second pass on a clean machine from the public addresses after each release (`git clone`, `./install.sh`).
- Measure a backup on a database of realistic size, and the *stress* video scenario on the Raspberry Pi 5.
- Document the local name (`dominet.local`) on a home network: avahi must announce on the LAN interface only (`allow-interfaces=`), otherwise a Docker bridge address is advertised; a name already taken gives `dominet-2.local`; some ISP boxes cannot change the DNS.

Code

Ideas, not decided
- A local assistant (a language model) that can also edit existing items reliably; today it reads, drives and creates only (`docs/assistant.md`).
- Hardware video decoding on arm64.
