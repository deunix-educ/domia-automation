# Measurements

Every figure that drove a decision, with the machine, the method and how to redo it. Use it as a baseline: a result far from these numbers means something changed.

**Reference machines.** *Target*: Intel Core i5-6500 (Skylake, 4 cores, no AVX-512), 7.8 GB RAM, Intel HD 530 (VAAPI), Debian 13. *Development*: 20 threads, 48 GB RAM, RTX 4060 8 GB, no usable VAAPI.

## 1. Feasibility spikes (phase R0)

| Topic | Verdict | Key figure |
|---|---|---|
| Server-rendered Leptos with hydration, WebSocket and translations | go | 16 of 16 browser checks; WASM 448 KB (190 KB gzip) |
| Time-series store, 1 million records | go | 1 680 writes/s; threshold query 99 ms per device |
| WHEP live view in WASM | go | 40 ms latency (VP8, local loop); real 720p live validated |
| H.264 decoding | go, with `ffmpeg` | 4 cameras: 22 % of the machine in software, 8 % with VAAPI |
| ONNX Runtime with YOLOv8n | go with reservations | 43 to 106 ms per image; detections identical to the reference model on 10 of 10 images |
| ONVIF against 4 real cameras | go | authentication, profiles, PTZ and IR-cut work; one camera absent from multicast discovery |
| Sandboxed Rhai scripts | go | 18 of 18; a hostile script is cut in under 45 ms |
| MQTT client | go | no loss at 2 000 msg/s; reconnection in 0.6 s |

## 2. Video load (phase R6)

Tool: `tools/bench-video.py`. Nothing shared with a running system: throw-away PostgreSQL, MediaMTX and ReductStore on shifted ports, no real camera.

**Protocol.** Simulated cameras are one 1080p, 15 fps, 3 Mbit/s H.264 clip encoded once and republished with `-c copy` (their own cost is 0.01 to 0.08 core in total). Three scenarios, 20 s warm-up then 60 s measured, a sample every 5 s: *decode only* (2 decoded frames per second); *full* (continuous recording in 60 s fMP4 segments, indexing and archiving, motion every 30 s, YOLO on each motion); *stress* (motion every 5 s, six times more YOLO). "Cores" counts the application, its decoding `ffmpeg` processes, MediaMTX, ReductStore and PostgreSQL together, in average whole cores.

| Target machine | Cameras | Cores | Memory (MB) | Machine busy |
|---|---|---|---|---|
| VAAPI, decode only | 4 | 0.46 | 673 | 15 % |
| VAAPI, decode only | 12 | 1.35 | 1 782 | 45 % |
| VAAPI, full | 4 | 0.55 | 863 | 18 % |
| VAAPI, full | 12 | 1.47 | 1 885 | 45 % |
| VAAPI, stress | 12 | 1.34 | 1 826 | 43 % |
| Software, decode only | 4 | 1.12 | 747 | 30 % |
| Software, full | 8 | 2.20 | 1 445 | 60 % |

What it says:

- A camera costs about **0.11 core** with VAAPI (0.12 in full mode) and **0.28 core** in software. VAAPI divides the video cost by about three (decoder `ffmpeg`: 0.31 against 0.98 core for 4 cameras; on a file, 3.5 s of CPU against 10.4 s for 100 s of video).
- **12 cameras in 1080p are held at 45 % of the target machine with VAAPI** (1.5 cores, 1.9 GB), 8 in software (60 %). Cost is linear, so half the machine stays free for the rest.
- YOLO is not the limiting factor: the application stays under 0.6 core even in stress at 12 cameras (about 105 ms per image on one core).
- Memory is about 150 MB per camera plus about 0.3 GB of base (PostgreSQL, ReductStore, the application).
- On the real machine, once VAAPI was enabled in the container, the application's CPU fell from 85-90 % of a core to about 36 % (41 to 49 % over the 48 h watch with every service running).

Limits: identical synthetic streams (real ones may cost more: 5 megapixels, some H.265 profiles not decoded by Skylake VAAPI, variable bitrate); no web, Zigbee, Grafana or MQTT traffic during the run; live WebRTC to browsers not measured (it depends on viewers; MediaMTX does not re-encode).

Redo it:
```
make fetch-model && cargo leptos build --release
python3 tools/bench-video.py --bin target/release/dominet --site target/site --model models/yolov8n.onnx \
  --bus crates/dominet-surveillance/fixtures/yolo/bus.jpg --workdir /tmp/bench --counts 1,4,8,12 --seconds 60
```
Use `--nice 19` on a shared machine and `FFMPEG_HWACCEL=off` for the software series.

## 3. MQTT and WebSocket load (phase R8)

Measured on the development machine, never on production.

- **MQTT**: `cargo test -p dominet-server --test throughput`: 50 devices, 2 000 messages/s sustained for 5 s against the development Mosquitto, no final state lost, publication refused cleanly (never blocked) when the client is saturated. 7 s.
- **WebSocket**: `cargo test --release -p dominet --test ws_load -- --nocapture` (variables `WS_LOAD_USERS`, `WS_LOAD_RATE`, `WS_LOAD_MESSAGES`, `WS_LOAD_P99_MS`).

| Clients | Rate | States delivered | Latency p50 / p99 / max | Process memory |
|---|---|---|---|---|
| 200 (25 accounts × 8) | 200 /s | 120 000 / 120 000 | 4 / 13 / 15 ms | 15 to 78 MB |
| 1 000 (125 accounts × 8) | 100 /s | 600 000 / 600 000 | 79 / 108 / 128 ms | 15 to 315 MB |

The memory includes the 1 000 test clients living in the same process. A debug build is about 100 times slower (p99 of 1.1 s with 200 clients): the default bound of the test accounts for it. A household has a handful of open browsers, far below these loads. Mosquitto drops messages beyond 1 000 queued per client unless `max_queued_messages` is raised (see the pitfalls page).

## 4. Production watch (phase R8, 48 hours)

13 services healthy, no automatic restart, no alert, all backups present. Memory 2.6 to 2.9 GB, load 1.3 to 3, database 58 MB growing by about 60 to 70 MB a day (about 3 readings per second). Solar-event automations ran at the computed time with the site's coordinates. 863 Rust tests and 29 E2E scripts (783 checks) on the image at the time.

## 5. Local assistant (phase R12)

Machine: development, llama.cpp b12078 with CUDA 12, context 16 384, temperature 0, reasoning disabled. Thresholds: a daily action card under 8 s, a configuration step under 30 s.

| Model | Mode | Generation (tok/s) | Action card, median (s) | Right tool |
|---|---|---|---|---|
| Qwen3-8B Q4_K_M | GPU | 16.2 | 1.8 | 96 % |
| Qwen3-8B Q4_K_M | CPU | 3.6 | 8.3 | 100 % |
| Qwen3-14B Q4_K_M | GPU | 3.9 | 7.4 | 92 % |
| Qwen3-4B Instruct Q4_K_M | GPU | 67.7 | 0.4 | 76 % |
| Qwen3-30B-A3B Instruct Q4_K_M | GPU | 21.2 | 1.2 | 80 % (failed the spike) |

End-to-end evaluation, 50 requests played through the real agent loop and tools against a test database with three deliberately confusing device names: **Qwen3-8B 49 of 50 (98 %) in 237 s, Qwen3-14B 48 of 50 (96 %) in 783 s, no wrong card in either** (threshold: 85 % and zero wrong card). The 8B is about three times faster for the same quality, so a single model was kept. The set is now 51 requests (49 pass). It is played with `cargo test -p dominet-agent --test eval -- --ignored` against a running `llama-server` (variables `AGENT_LIVE_URL`, `AGENT_LIVE_MODEL`, `AGENT_EVAL_OUT`); raw results are in `docs/bench/`.

## 6. Build and test times

- `make ci` (format, lints, tests, WASM, deny, audit): about 4 min when warm; the `pre-push` hook replays it on a clean copy of the pushed commit in about 16 min (956 to 1 029 s over recent pushes).
- Tests are bounded to 8 threads; at 20 threads `PoolTimedOut` appears about one run in three.
- Release WASM about 3 MB; development WASM 48 MB. Image about 713 MB.
- Disk on the target: the application's own growth is small (about 65 MB a day of history); video dominates, so size the recordings volume first.
