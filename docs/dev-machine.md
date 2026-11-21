# Development machine

This page prepares a machine for developing the application. Develop preferably on a fast machine (at least 8 cores and 16 GB); the machine that runs the application every day can be far more modest (see [Measurements](measures.md)).

## Install

```bash
git clone git@github.com:deunix-educ/domia-automation.git
cd domia-automation && tools/setup-dev-host.sh --ci
```

`tools/setup-dev-host.sh` checks the system packages (and prints the `apt` command if some are missing), installs Rust at the pinned version and the gate tools (`cargo-deny`, `cargo-audit`, `sqlx-cli`, `cargo-leptos`), enables the `pre-push` hook, starts the development stack and applies the migrations; with `--ci` it then runs `make ci`. `tools/check-tools.sh` prints the versions of the expected tools.

Prerequisites it cannot install: Docker (with the `compose` plugin and your user in the `docker` group) and Firefox (end-to-end tests).

## Run the application in development

```
make dev-up        # development stack: PostgreSQL, MediaMTX, Mosquitto, ReductStore
make dev-run       # http://127.0.0.1:8080 (DISCOVERY=1 make dev-run: with the discovery role)
```

`make dev-run` (`tools/dev-run.sh`) builds the binary and the interface if they are missing, applies the migrations, creates the master key and an administrator account on first launch, then starts `dominet serve`. The credentials are kept in `deploy/.env.dev-run` (ignored by git, mode 600): never commit it and never print it.

## The gate before commit and before push

- `make ci`: format, lints as errors, tests, WASM, `cargo deny`, `cargo audit`, SQL metadata. The development stack must be running (`make dev-up`).
- `make ci-full`: adds the Docker image build and the browser tests on that image; run it before a milestone or when the `Dockerfile` or a native dependency changes.
- The `pre-push` hook (`make hooks`) reruns `make ci` on a clean copy of the pushed commit. It uses the development stack: do not stop it during a push. If your host drops the connection meanwhile: `git -c core.sshCommand="ssh -o ServerAliveInterval=30" push`.

There is no automatic CI: the gate is local (see [Decisions by theme](decisions.md)).

## Disk space

`target/` grows fast (the hook builds a clean copy in `target/prepush`, the test profile in `target/debug`) and can pass several hundred GB. `rm -rf target/prepush` frees the largest part at no cost (it is rebuilt on the next push). Never build on the production machine.
