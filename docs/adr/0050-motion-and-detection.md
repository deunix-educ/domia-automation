# 0050 — Motion and object detection

- Status: accepted
- Date: 2026-11-08

## Context
The reference (`detect_motion_camera`, `run_yolo_detection`, `OnnxBackend`): for each camera **in `motion` mode**, every 30 s, it reopens the stream, reads ~20 frames, compares the first and the last (gray, 21 × 21 blur, `absdiff`, threshold 25, 5 × 5 opening, pixel ratio > `MOTION_THRESHOLD` 0.01), creates a `MotionEvent`, publishes on MQTT, notifies (per-camera delay 600 s) then runs YOLOv8n if the camera enables it (one frame, the camera's classes and confidence, annotated image, `DetectionEvent`, MQTT, 300 s notification). Decisions: a single `FrameSource` with a sliding window, the model, measurements.

## Decision
- **Motion in pure Rust** (`dominet_surveillance::motion`, no dependency on OpenCV): the same algorithm, with OpenCV's details reproduced (14-bit integer gray, separable blur with reflect-101 borders, σ derived from the kernel size, opening with neutral borders). **Parity measured** against OpenCV 5 on 8 synthetic image pairs (`tests/motion_parity.rs`): maximum gap **0.0002** of the ratio (test tolerance 0.0005); same decisions at the 0.01 threshold for every case. OpenCV's 8-bit blur is fixed-point, not reproduced bit for bit.
- **Sliding window**: the `FrameHub` keeps the last 20 images of each camera; every `MOTION_INTERVAL_SECS` (30), the first and the last are compared — **only two images are decoded and blurred**. No stream is reopened (the reference: 2 to 4 s of set-up each round). At 2 frames/s the window covers ~10 s (the reference: ~2 s at 10 frames/s), so a slow motion is easier to see; a deliberate deviation, adjustable by the threshold. At least 5 images and a last image younger than 15 s, otherwise no verdict.
- **Consequences of a motion**: `MotionEvent`, MQTT `dominet/surveillance/<uuid>/motion` (`camera_uuid`, `camera_name`, `confidence` to 4 decimals, `timestamp`), e-mail + SMS `Motion detected — <name> (<place>): x.x%` with a per-camera delay `MOTION_ALERT_COOLDOWN`, then YOLO. The `video` role publishes through the `mqtt` role's client if it runs in the process, otherwise through its own (`<client_id>-video`).
- **YOLO** (`dominet_surveillance::yolo`, `ort` pinned `=2.0.0-rc.15`, ADR 0009): same input / output / thresholds as the reference detector (640 × 640, score ≥ 0.10 then NMS 0.45 class-agnostic, the camera's threshold and classes applied afterwards). A session pool with one compute thread (`YOLO_WORKERS`, default 1), loaded **at the first detection** (the model does not occupy memory if turned off). Input: the **latest `FrameSource` image** (JPEG at most 1280 wide), not a raw stream image like the reference: compression and reduction add a slight gap, negligible for class, box and score (see parity). **Parity measured** on 4 images (Ultralytics' public `bus.jpg` sample and three variants: resized, cropped, without object): **17/17** detections of the reference found again (same class, IoU > 0.9, score gap < 0.02), none in excess.
- **Global setting**: `DetectorConfig` (one row; `enabled`, `save_frames`, `model_path`), modified in the interface (`/surveillance/events`, administrators). Off by default. `save_frames`: annotated image `<uuid>_<YYYYMMDD_HHMMSS>.jpg` in `DETECTIONS_DIR`, served by `GET /surveillance/detections/<id>/frame/` (any signed-in account). **Deviation**: boxes are drawn, **not the text** (no embedded font).
- **Events page** `/surveillance/events` (any signed-in account): recent motions and detections (server time), the annotated image when it exists.
- **Model**: `yolov8n.onnx` **is not committed**; `make fetch-model` exports it from the official Ultralytics weights in a throw-away container (same version and options as the reference file: `ultralytics` 8.4.82, opset 12, simplify; licence AGPL-3.0). `make image` fetches it if absent and the `Dockerfile` embeds it (`/models`). Without a model the parity tests say so and verify nothing; detection logs "model unavailable".

## Verification
- Unit: integer gray, normalised kernel and σ of OpenCV, reflect-101, blur of a flat image and of a point, ratio (squares, specks and thin lines erased by the opening, strict threshold), NMS and box scaling, IoU, pre-processing, annotation, class and confidence filter, per-camera delay.
- Parity: `motion_parity.rs` (OpenCV) and `yolo_parity.rs` (the reference detector).
- Integration (real `ffmpeg`, real model, development Mosquitto, fake SMTP): a fixed source (no motion) and an oscillating source (motion, event, MQTT, a single e-mail despite two motions), no window; detector off / YOLO off / unwanted class (nothing), wanted class (event, readable annotated image, MQTT, e-mail), without `save_frames` (no file).
- End-to-end `events` (a real H.264 stream of the bus image, oscillating brightness): quantified motion, "bus" detection with an annotated image served as JPEG, page, detector turned off from the interface.

## Limits
- Load measurements (CPU / RAM, cameras held) are in [Measurements](../measures.md): on the target machine, **12 1080p/15 cameras in full mode (recording, archive, motion, YOLO) = 45 % of the machine with VAAPI**, 8 cameras = 60 % in software.
- `motion` mode still does not trigger recording (see ADR 0049); YOLO only triggers on motion (no periodic sampling, although `DetectionTrigger::Periodic` exists in the schema).
- VAAPI decoding and the GPU are not used for YOLO (ADR 0009).

## Addendum: Docker image
The precompiled ONNX Runtime library used by `ort` is C++: building needs `g++`, running `libstdc++6`, and the whole needs glibc ≥ 2.38. The image therefore moves from Debian 12 to **Debian 13**, identical to the target. Discovered by `make ci-full`, which had not been rerun after the `Dockerfile` change.
