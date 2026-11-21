# 0017 — Docker image with `cargo-leptos` and end-to-end tests on the image

- Status: accepted
- Date: 2026-11-01

## Context
Until this point the image only held the server binary (no WASM/JS/CSS files: pages rendered but did not hydrate). A complete image is now needed. A defect met at the end of the previous work imposes a requirement too: the server (rendered HTML) and the WASM (hydration) must be compiled **the same way**, otherwise the interface displays but stays inert (disabled buttons, no message); neither `clippy` nor the Rust tests see it.

## Options
1. **`cargo leptos build --release` in the image**, with `cargo-chef` to cache dependencies (server and WASM) — chosen.
2. Build the WASM outside the image (development machine) and copy it — not reproducible, machine dependent.
3. File-name hashing (`hash-files`) for a long cache — discarded for now: the fingerprint file must sit next to the binary, which adds one more dependency between development binary, `cargo leptos` and image (the class of defect just fixed); `/pkg/*` stays `no-cache` (revalidation, a few-byte 304).
4. **arm64** — see Decision.

## Decision
- **`Dockerfile`**: `cargo-leptos` 0.3.11 (pinned) and `cargo-chef` installed in a base layer; two `cargo chef cook` (server, and WASM with the same target, the same `wasm-release` profile and the same `target/front` folder as `cargo leptos build --release`); `cargo leptos build --release` produces the binary and `target/site`; the final image (Debian slim, user 10001) contains `/usr/local/bin/dominet` and `/app/site` (`SITE_ROOT`). `.cargo/config.toml` is copied from the first stage: it carries the settings common to server and WASM. `cargo-leptos` downloads `wasm-bindgen` (version of `Cargo.lock`) and `wasm-opt` during the build: network needed. Result: a **1.3 MB** WASM (20 MB in development), a 137 MB image.
- **Smoke test** (`ci/image-smoke.sh`): health, `/readyz` without a database, non-root, clean stop, **and** `/login` (200, nonce CSP, expected WASM file, hydration markers), `/pkg/*` (200, `application/wasm`, gzip, `no-cache`, plausible size).
- **End-to-end tests on the image** (`tools/e2e.sh --image`, `make e2e-image`): a container on the host network, a throw-away database created then dropped, headless Firefox with a temporary profile, the scripts of `tests/e2e/`. It is the only test that hydrates the page with the **production** binary and WASM; verified by forcing a failure (exit code 1, "Interactive interface: 0 click") when static files are missing. The same launcher without `--image` serves the development binary (`make e2e`).
- `make ci-full` = `make ci` + `image-smoke` + `e2e-image`, to run before a milestone and when the `Dockerfile` or the build chain changes (`DOMINET_PREPUSH_CMD='make ci-full' git push` applies it to a push). `make ci` stays the ordinary gate (ADR 0015).
- **arm64 not built.** The development machine has no emulation (`exec format error`) and configuring `binfmt`/QEMU needs root; it is a non-blocking point until a Raspberry Pi 5 is targeted. To resume with a native arm64 machine: the `Dockerfile` has no architecture dependency, only the downloaded `wasm-bindgen`/`wasm-opt` versions must exist for `aarch64`.
- No `HEALTHCHECK` in the image (the slim image has no `curl`): the `/healthz` probe is declared in the production `docker-compose`.

## Consequences
- A change to the `Dockerfile`, `.cargo/config.toml`, `Cargo.toml` (profiles, Leptos metadata) or tool versions is validated with `make ci-full`.
- A long cold first build (cargo-leptos, server and WASM dependencies); afterwards only project code is rebuilt: a one-line change rebuilds the image in about 2 min 30 (measured).
- Tools downloaded during the build (`wasm-bindgen`, `wasm-opt`) are a network dependency of the build; an internal base image or a cache of these binaries could replace them.
