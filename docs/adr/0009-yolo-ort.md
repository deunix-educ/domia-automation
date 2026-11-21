# 0009 — Object detection: YOLOv8n through `ort`

- Status: accepted
- Date: 2026-10-31
- Spike verdict: **go with reservations** (`ort` has no stable release; Intel GPU not evaluated)

## Context
Reproduce the reference detector: YOLOv8n ONNX (12.9 MB), input 640×640 NCHW float32 RGB/255, output `[1,84,8400]`, score ≥ 0.10, NMS IoU 0.45 (class-agnostic). Spike: `ort` 2.0.0-rc.15, prebuilt ONNX Runtime 1.31.0; reference: `onnxruntime` 1.31.1 CPU. Machine: Intel Core i5-6500 (Skylake, 4 cores, AVX2).

## Measurements (median of 60 passes, 640×640 input image)
| Intra-op threads | `ort` inference (Rust) | Reference inference | Rust total (pre + inference + post) |
|---|---|---|---|
| 1 | 105.6 ms | 109.0 ms | 112 ms |
| 2 | 60.0 ms | 62.4 ms | 67 ms |
| 4 | 42.8 ms | 41.7 ms | 50 ms |

- Rust pre-processing (direct bilinear resize + normalisation + NCHW) ≈ 4.5 ms; post-processing + NMS ≈ 2 ms.
- **Parity: 10/10 detections** of the reference found again (same class, box IoU > 0.9, score gap < 0.02) on 5 images (COCO "bus" image: 4 people + bus + umbrella; 3 images from real cameras).
- OpenVINO (trial through `onnxruntime-openvino`): **CPU** provider 38.6 ms against 41.7 ms (+ ≈ 8 %, negligible); **GPU** provider unusable here (Intel OpenCL runtime `intel-opencl-icd` absent) → CPU fallback. Not evaluated on the iGPU.
- Test binary: 31 MB (ONNX Runtime statically linked), dynamic dependencies `libstdc++`, `libgcc_s`, `libm`, `libc`. Session creation ≈ 75 ms.
- Prebuilt ONNX Runtime binaries for `ort`: **x86_64-linux-gnu** and **aarch64-linux-gnu** (CPU); **no musl** → glibc image (Debian slim).

## Decision
- `dominet-surveillance::yolo`: `ort` (CPU), pure-Rust pre/post-processing, same contract as the reference (`score ≥ 0.10` before NMS, the user threshold `yolo_min_confidence` applied by the caller).
- **Exactly pinned version** (`=2.0.0-rc.15`): `ort` only has release candidates (moving API). An upgrade is an explicit task with a rerun of the parity test. Features: `std`, `ndarray`, `download-binaries`, `tls-rustls`, `copy-dylibs` (the default `tls-native` would need OpenSSL, absent).
- **A pool of workers with 1 intra-op thread each**, rather than a multi-thread session: it is the most efficient per core (0.112 core·s per inference against 0.20 at 4 threads). Throughput target: **≈ 9 inferences/s per dedicated core**.
- **Budget**: YOLO sampled at 1–2 frames/s per camera (not at the video rate); 4 to 5 cameras at 2 frames/s ≈ 1 core of YOLO + ≈ 8 % of the machine for VAAPI decoding (ADR 0003). The number of workers and the frequency become settings (`AppSetting`).
- glibc Docker image, `models/yolov8n.onnx` copied into the image, path through `YOLO_MODEL_PATH`.
- Regression test: the reference COCO image and its detections (no sensitive data) live in `tests/fixtures/yolo/`; the parity test compares with these frozen values.

## Consequences
- Single-frame latency is fine for event detection, not for tracking at video rate: consistent with the use (alerts, `DetectionEvent`).
- The potential gain of the iGPU (HD 530, 24 EU) is probably modest against an AVX2 CPU and competes with VAAPI. `intel-opencl-icd` **does not exist in Debian 13**: the trial would need Intel's repository. **Abandoned** unless a real capacity need appears.
- arm64 (Raspberry Pi 5): a prebuilt ONNX Runtime binary exists, performance **not measured**.

## Pitfalls found
- `ort` pulls `native-tls` by default to download its binaries at build: without `libssl-dev`, use `tls-rustls`; the build needs network access to `cdn.pyke.io` (plan for CI and the Dockerfile, or pre-download with `ORT_LIB_LOCATION`).
- `SessionBuilder` errors are not `Send`: do not propagate them as they are into `anyhow` (convert them to a string).
