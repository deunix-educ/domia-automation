# ADR 0058 — MCP server built into the binary: `rmcp`, session-less HTTP, `dominet-mcp` crate

Status: accepted. Decisions of the owner: a built-in server on `/mcp`, LAN and VPN only, editable Rhai scripts, messaging excluded, every active device controllable.

## Context
An assistant (Claude Code, Claude Desktop or the local assistant) must be able to read, control and configure DomiNet under the same rules as the web interface, without ever seeing a secret. The audit ([MCP audit](../mcp-audit.md)) shows that the 107 server functions of the interface all follow the same pattern (`AppState`, access check, logic on the repositories): the logic is extractable, nothing has to be invented on the API side.

## Options
1. **A separate `stdio` binary** that calls back the `/sf/...` functions: duplicates the schemas, depends on the internal format of server functions, to install on every workstation. Discarded.
2. **A separate service (Node)** that calls `/mcp` with a token: one more deployment, one more secret and one more network surface.
3. **A server built into the `dominet` binary, Streamable HTTP transport** (chosen).

For the mounting: (a) `dominet-server` depends on `dominet-mcp` and passes it a services trait; (b) `dominet-mcp` depends on `dominet-server` and **the binary** mounts its routes. For the SDK: `rmcp` (official) or a hand-written JSON-RPC.

## Decision
- **`dominet-mcp` crate**, depending on `dominet-server` (`AppState`); the `dominet` binary passes `dominet_mcp::routes` to `server::serve`, which adds them to the router **under the same middlewares** (log, request id, 30 s timeout, security headers). No circular dependency, no 35-method trait to maintain beside the services. A non-browser client (no `Origin`) is not concerned by the CSRF check.
- **`rmcp` SDK**, version **pinned** exactly: the protocol moves fast, an upgrade is an explicit task. Apache-2.0 licence, accepted by `cargo deny`; no new `cargo audit` warning.
- **Session-less**: `legacy_session_mode = false` and plain JSON responses (`json_response`). Each request is self-contained, which suits tools that answer in a few milliseconds and keeps the server stateless per client. A client of the previous protocol revision also works without a session (tested).
- **DNS-rebinding protection**: the `Host` header must be `localhost`, `127.0.0.1`, `::1` or listed in `ALLOWED_HOSTS`; otherwise 403.
- **Disabled by default**: `MCP_ENABLED=false`; enabled without a valid token, start-up is refused.
- **Authentication** `Authorization: Bearer` checked by a middleware before the SDK; neither the header nor the token is logged (see [ADR 0059](0059-mcp-access-tokens.md)).
- **Reading the configuration in the `read` scope**: `config_list`, `config_get` and `config_schema` do not require `configuration`; secrets stay masked. A diagnostic token can read everything and change nothing.
- **A single write path**: the interface and MCP save through `dominet_server::config_service`, which carries the checks beyond the domain draft (a script that compiles, an existing scene, name conflicts) and the reload of the live parts. The MCP `dry_run` calls the same checks: it announces the refusals of a real write.
- **Tools by domain** (about 35, not 110); each tool calls the **same service function** as the web page. No business rule in `dominet-mcp`.
- **Rhai script is a scope of its own**: any `config_upsert` whose payload contains Rhai code (the `script` action of an automation, the handler code of an MQTT source) requires the `administration` scope, not `configuration`.

## Consequences
- In session-less mode `rmcp` calls the service factory **on every request** (sometimes twice: tool-schema lookup, then handling): the tool table is rebuilt at each call. Negligible for one tool, to measure as the catalogue grows.
- The logic of the server functions had to be **extracted** into `dominet-server` in the form `service(&AppState, &Actor, input)`; the interface benefits.
- The MCP specification and `rmcp` evolve: pinned version, isolated crate, snapshot of the tool schemas in the tests.
- The router's 30 s timeout also bounds the tools; a tool that must last longer gets its own route.
- Not handled here: OAuth 2.1 (useless on LAN and VPN), a `dominet mcp --stdio` mode (possible later on the same crate).
