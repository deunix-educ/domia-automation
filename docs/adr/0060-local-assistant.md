# ADR 0060 — Local assistant: engine, models and limits

Status: accepted. Measurements: `docs/bench/agent-bench-2026-11-17.md`.

## Context
The assistant answers in the interface and calls the tools of the MCP server. The architecture decisions: the agent lives in `dominet`, the LLM is self-hosted on another machine, `llama-server`, no remote API, `Admin` role only. This work package measures what the hardware allows: the test machine is a development workstation (i9, 46 GB RAM, 8 GB RTX 4060), which also served as the inference machine for the trial.

## Options measured
Four Qwen3 models in Q4_K_M, with the **real** system prompt and the **real** tools (everyday profile: 29 tools, 4,930 tokens; `confirm` and `dry_run` removed), 30 requests in French, temperature 0, on GPU (layers split by `llama-server`) and on CPU only.

| Model | Mode | Generation (tok/s) | Action card (s) | Right tool | Remark |
|---|---|---:|---:|---:|---|
| 4B Instruct 2507 | GPU | 67.7 | 0.4 | 76 % | too many questions instead of acting |
| **8B** | **GPU** | **16.2** | **1.8** | **96 %** | fits entirely in 8 GB |
| 8B | CPU | 3.6 | 8.3 | 100 % | at the limit of the 8 s threshold |
| **14B** | GPU (partial) | 3.9 | 7.4 | 92 % | spills into RAM, slow but under 30 s |
| 14B | CPU | 1.9 | 15.1 | 96 % | |
| 30B-A3B Instruct 2507 | GPU | 21.2 | 1.2 | 80 % | **invented devices without calling a tool** (failed spike) |
| Ministral-3 8B (Mistral) | GPU | 13.0 | 1.3 | 96 % | fast, but invents a scene name ("Switch everything off") on "Turn everything off"; spike answer unfaithful to names |
| Ministral-3 14B (Mistral) | GPU (partial) | 4.8 | 3.3 | 88 % | contradiction in the spike; 40 % success on ambiguous requests |
| Mistral-Small-3.2 24B | GPU (partial) | 2.0 | 9.9 | 92 % | too slow (above the 8 s threshold), no more reliable than Qwen3-14B |

The prefix (system prompt and tools) is cached: a warm turn costs 0.02 to 0.5 s of prefill.

Mistral measurements: `docs/bench/agent-bench-2026-11-17-mistral.md`. On ambiguous requests ("Close it", "Turn everything off"), the Mistral models first search with a read tool instead of asking (the set counts this as a failure, although a search has no effect); the Qwen3 ask the question. The Mistral speed measurements were slightly disturbed by repository checks run in parallel.

## Decision
- **Engine**: `llama-server` (llama.cpp, CUDA), `--jinja` for tool calls, one resident model per instance, 16k context.
- **Everyday model** (`AGENT_MODEL`): **Qwen3-8B Q4_K_M**. Action card in under 2 s on an 8 GB GPU, 96 % right tools, questions asked on every ambiguous request.
- **Configuration model** (`AGENT_MODEL_ADMIN`): initially **Qwen3-14B Q4_K_M**; **replaced by the same model as the everyday one** (see "Owner's decision" below). The setting stays, empty by default (= `AGENT_MODEL`), for a future trial.
- **Mistral models compared and not chosen** (8B, 14B, Small 24B): no more reliable than the Qwen3 of the same size, slower for the 24B, and they reword or invent names. Mistral's 8B is a serious fallback for everyday use (96 %, 1.3 s).
- **Discarded**: the 4B (misses too many calls) and the 30B-A3B (fast but answers without reading the house: this defect, serious for a diagnostic assistant, makes the model unusable without finer measurement).
- **System prompt**: it must say to **act first** (search with the read tools) and to ask a question only when the request does not say what to act on; the first, over-cautious version dropped the 4B to 52 %.
- **Without a GPU**: the 8B stays usable (8 s per card, 3.6 tok/s) but at the threshold; the 4B (6 s) is the fallback.

## Limits of the measurement
Only 30 requests: a 4-point gap is a single case. First turn only. GPU/CPU gaps of the same model (the 4B goes from 76 % to 92 %) come from numerics and show the noise. The 50-request set, injections and long configuration requests were measured afterwards.

## Consequences
- `make agent-bench` replays the measurement at every change of machine or model; the table is versioned in `docs/bench/`.
- Model weight: 5 GB (8B), resident on the 8 GB GPU; a single `llama-server`.
- Assistant quality depends as much on the tool descriptions and the prompt as on the model; the 4,930 tokens of tools are a fixed cost, covered by the cache.

## End-to-end evaluation
A set of **50 requests** played end to end by the real agent loop (`make agent-eval`, `docs/bench/agent-eval-2026-11-17.md`), threshold **85 % and no card for an ambiguous request or an injection**: **Qwen3-8B 49/50 (98 %)**, **Qwen3-14B 48/50 (96 %)**, zero wrong card for both. The 14B is **not more reliable** than the 8B on this set (configuration 9/9 for both) and **2 to 3 times slower** (13.5 s against 6.2 s per configuration request); it is better on ambiguous requests before the anti-promise reminder (8/8 against 5/8). The two-model design therefore rested mostly on caution; **a single model (the 8B) is enough on these measurements**. The set is small (a 2-point gap = 1 case) and the test house simple.

## Owner's decision
**A single model: Qwen3-8B Q4_K_M**, for every regime. `AGENT_MODEL_ADMIN` is empty by default and means "the same as `AGENT_MODEL`"; the 14B stays measurable with `make agent-eval` if models evolve. Consequences: a single `llama-server` instance, 5 GB to download, configuration requests in about 6 s.

## Outcome after trials
The assistant reads, controls and creates reliably; **modifying or renaming an existing element is not reliable with an 8B model** (and a device's name is also its MQTT topic), so the assistant does not offer it: that is done from the administration pages. The evaluation set is now 51 requests, 49 passed, no wrong card. The assistant phase was then set aside.
