# 0021 — Page render guard: abort and retry a frozen render

- Status: accepted (workaround, to remove when Leptos fixes the defect)
- Date: 2026-11-02

## Context
The `realtime` end-to-end test failed about 1 time in 10 on the Docker image (a page that never finishes: `goto` or `fetch` without an answer, 30 s script timeout). The investigation, summarised:

- **The server is not broken**: during the freeze, `/healthz`, `/` and `/admin/settings` answer in 0.0 s to another client with the same cookie. Only one request is lost.
- **Reproduced without a browser** (`tools/stress-cold-render.sh`): after a cold restart, 3 concurrent requests on the same page freeze 1 request in 1 to 5 % of cases (`/admin/settings` as well as `/`); 0 out of 100 with one request at a time, 0 out of 150 on an endpoint that only queries the database (`/api/auth/me`, eight concurrent requests), 0 out of 100 once the same page has already been rendered once. Production image, debug binary and optimised binary: same behaviour as soon as renders overlap when cold.
- **All server threads sleep** during the freeze (0 processor ticks in one second): it is neither a loop nor a held lock, it is a wait that is never woken up in the `Suspense`/`Transition` rendering of Leptos 0.8.23 (the last stable version; 0.9 only exists as a candidate).
- **Independent of the statistics collector**: active or nearly idle (1 s or 3,600 s) gives the same rate (2/40 against 1/40), and the pitfall "several Leptos apps rendered in parallel disturb each other" was already known (the `ui.rs` test serialises them).
- The `panics` in the log ("reactive value that has already been disposed", in `tr!` of the language selector) are a **consequence**: when the client disconnects, the request is dropped, its reactive context is released and an orphan task of the frozen render wakes up. They also appear when a client cuts off during a healthy render.
- Hypotheses tried and discarded: a `new_blocking` resource in the layout (replaced by `new`: freeze unchanged), `on_cleanup` of the real-time subscriptions (the faulty read is `leptos-fluent`'s, not ours), sorting by Firefox headers (`Accept-Language`…), build mode.

## Options
(a) Wait for a Leptos fix — leaves 1 to 5 % of frozen pages in cold concurrent-render situations.
(b) Move to the 0.9 candidate — an uncontrolled API change in the middle of a phase.
(c) Rewrite the pages to avoid `Transition`/resources — enormous, and the exact cause is not isolated.
(d) **Guard: bound the render of a `GET` and retry it** — a `GET` changes nothing, a new render almost always succeeds — chosen.

## Decision
`web/render_guard.rs` wraps the interface service (pages and static files): for a body-less `GET`/`HEAD`, the response must arrive within **5 s** (a normal render takes a few tens of milliseconds); otherwise the render is abandoned and the request, copied identically (method, address, headers, extensions: session, peer address), is retried, **at most 3 times**; after that the answer is a **503 with `Retry-After: 1`**. `POST`s (server functions), requests with a body and WebSockets are never timed out or retried. Every abandonment is logged (`WARN … page render without answer: abandoned and retried`).

Checked by `tools/stress-cold-render.sh`: 300 cold starts on the image with the guard, 0 anomalies, 1 render retried and successful (without the guard: 1 to 5 frozen requests per 100 starts).

## Consequences
- A frozen render costs 5 s instead of an indefinite hang; a genuinely slow render (> 5 s, saturated database) is retried needlessly, up to 3 times (read-only, bounded).
- The log contains `panics` from orphan tasks after a retried render (noise, no effect on other requests: confirmed by the load test).
- To remove when Leptos fixes the defect: delete `render_guard`, rebuild the image and require **0 anomalies and 0 retried renders** from `tools/stress-cold-render.sh` over several hundred starts before concluding.
- The defect has probably been present from the start; the end-to-end tests escaped it because pages were rendered one at a time.
