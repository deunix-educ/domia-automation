# Local assistant (R12)

The **assistant** answers in French, inside the interface, to requests such as "Which shutters are still open?" or "Close them". It runs **at your place**: the model is a self-hosted `llama-server`, no remote API, no data leaves your network. Roadmap: `ROADMAP-agent.md`; decision and measurements: [ADR 0060](adr/0060-local-assistant.md); evaluation: [50 requests](bench/agent-eval-2026-11-17.md).

!!! info "Administrators only, off by default"
    The **Administration › Assistant** page (`/admin/assistant`) is reserved for the administrator role; it only answers when `AGENT_ENABLED` is true.

## What it does, and what it never does alone
- **Read**: device state, history, logs, alerts, configuration (never a secret).
- **Act**: the model **proposes** an action, shown as a **card** ("Close the living-room shutter"); you click **Confirm** or **Cancel**. Nothing runs without your click.
- **Configure** (the "configuration" request type): for an automation, the card also shows a before / after YAML diff.
- **Undo**: every confirmed change is written to the audit log (type `agent`) with your request, the previous state and the way back; the "Changes made by the assistant" list undoes it in two clicks. Not undoable: device commands, scenes run, notifications, and anything involving Rhai code.

## Install the model
On the inference machine (a PC with an 8 GB graphics card is fine; without a GPU it works but takes about 8 s per answer), with a llama.cpp `llama-server` already installed:
```bash
make agent-install LLAMA_DIR=/path/to/llama.cpp  AGENT_HOST=0.0.0.0
make agent-install LLAMA_DIR=... DRY=1     # shows what would be done, does nothing
```
The script downloads **Qwen3-8B Q4_K_M** (5 GB, resumable) into `~/IA/models` and creates the `dominet-llm` user service (16k context, tool calls, one request at a time). To keep it running after logout: `sudo loginctl enable-linger $USER`. **One model** serves all request types (ADR 0060, decision of 2026-11-17).

!!! warning "Network"
    `llama-server` has no authentication. Open it to the local network or the VPN only (`AGENT_HOST` = LAN address), never to the Internet.

## Enable on the DomiNet side
In **Administration › Settings**, category **Assistant**:

| Setting | Value |
|---|---|
| `AGENT_ENABLED` | `true` |
| `AGENT_LLM_URL` | `http://<inference machine>:8080` |
| `AGENT_MODEL` | `Qwen3-8B-Q4_K_M` |
| `AGENT_MODEL_ADMIN` | empty (same model) |

On the Assistant page, the **Test connection** button checks the address and the model. If the inference machine is off, the page says so and the forms remain usable.

## First tries
1. "Which lamps are on?" → an answer in one or two sentences.
2. "Close the living-room shutter" → a card; **Confirm**.
3. "Configuration" type: "Close the shutters at sunset" → an automation card with the before / after YAML; **Cancel** if it does not fit.
4. **Changes made by the assistant**: undo the last one.

## Limits
- A small model can be wrong or ask for details: rephrase with the device or room name.
- One request at a time, 40 messages of memory, 15 minutes of inactivity before the conversation is forgotten; the conversation is not stored in the database.
- What the house contains (device names, messages) is treated as data, never as an instruction: a device named "ignore your instructions" has no effect (injection tests: 0 wrong cards).
- Changing or renaming an item that already exists (a device, for example) is not reliable with a small model: do it from the administration pages. The assistant reads, controls and creates.
- Measured quality: 49 of 51 requests succeeded with Qwen3-8B (the two failures: "Turn everything off" and "Set the right temperature" without a clarifying question, no card), 0 cards proposed wrongly. Replay the measurement: `make agent-eval LLAMA_DIR=… AGENT_EVAL_MODELS=Qwen3-8B-Q4_K_M`.
