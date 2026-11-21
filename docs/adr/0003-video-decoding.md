# 0003 — Video decoding: `ffmpeg` subprocess behind `FrameSource`

- Status: accepted
- Date: 2026-10-31
- Spike verdict: **go** for `ffmpeg` (VAAPI when available, software otherwise); **no-go** for `openh264` as the default

## Context
Motion detection, YOLO and snapshots need decoded images. No mature 100 % Rust H.264 decoder exists (`retina` does not decode). Candidates: an `ffmpeg` subprocess (raw-frame pipe), `ffmpeg-next` (C binding, needs the `libav*-dev` headers, absent from the host, not evaluated), `openh264` (Cisco, software). The spike plus an `ffmpeg` bench ran on an Intel Core i5-6500 (Skylake, HD Graphics 530), `ffmpeg` 7.1.5, VAAPI driver `iHD` 25.2.3, 30 s clips captured without re-encoding from real cameras (720p15, 360p15 sub-stream, two 1080p models).

## Measurements (CPU as % of one core, per stream, per second of video)
From a file, at full speed (no network):

| Stream | Software `-threads 1`, 5 fps → 640×640 RGB | VAAPI decode only | VAAPI + 5 fps + `hwdownload` + software scale |
|---|---|---|---|
| 1080p camera A | 7.6 % | 1.2 % | 3.5 % |
| 720p15 camera | 9.4 % | 0.9 % | 2.5 % |
| 1080p12 camera B | 5.8 % | 1.1 % | 3.5 % |
| 360p sub-stream | 4.2 % | 0.7 % | 1.2 % |

Live (RTSP/TCP from the cameras), **4 simultaneous streams**, 5 fps to 640×640: software `-threads 1` = **87 % of a core** (22 % of the machine); VAAPI + `hwdownload` + software scale = **32 %** of a core (8 % of the machine). No failure. Latency of the first image with a cold-started `ffmpeg`: **2.1 to 4.3 s** (waiting for the key frame). Software decoding of every frame, for reference: 1080p ≈ 5–9 % of a core in bursts.

`openh264` (1 thread, frame by frame): 720p = **19 %** of a core (about 2× software `ffmpeg`); **failures on two camera families** (183 errors out of 211 access units, "no parameter set"; `ffmpeg` decodes them without error) even after regrouping into access units. Cause not elucidated in the time available; extra integration cost: RTP depacketising (`retina`) and no hardware decoding.

In a non-root Docker container (`debian:trixie-slim` + `ffmpeg` + `intel-media-va-driver`), VAAPI decoding works with `--device /dev/dri` and the host's `render` group added to the user.

## Decision
- A `FrameSource` trait; default implementation: **one long-lived `ffmpeg` per camera**, reading the **MediaMTX re-stream** (not the camera directly), writing fixed-size raw frames at N fps to stdout (`-vf fps=N,scale=W:H,format=rgb24`); the process is supervised (restart with backoff) and its frames are read in batches.
- **Opportunistic hardware acceleration**: at start-up, probe `ffmpeg -hwaccel vaapi … -f null`; if it succeeds → `-hwaccel vaapi -hwaccel_output_format vaapi` then `fps=N,hwdownload,format=nv12,scale=…`; otherwise fall back to software `-threads 1`. On a Raspberry Pi 5 (no hardware H.264 decoding) the software fallback is what runs.
- On-demand snapshots: do not start a cold `ffmpeg` (2–4 s); serve the latest frame of the already active `FrameSource` or, with no active stream, decode one key frame (`-skip_frame nokey`, about 2 % of a core).
- Docker: the image ships `ffmpeg` and `intel-media-va-driver`; the production Compose adds `devices: [/dev/dri:/dev/dri]` and `group_add: ["${RENDER_GID}"]` (the host `render` group GID, read by `make prod-init`).
- `openh264` and `ffmpeg-next` remain possible later implementations of `FrameSource`, not chosen by default.

## Consequences
- The cameras' CPU budget is small (about 8 % of the machine for 4 cameras with VAAPI, 22 % in software): **the bottleneck will be YOLO**, not decoding.
- The real stream resolution sometimes differs from the ONVIF profile: one camera streams at **1280×720** while its profile announces 1920×1080; on another, `/stream` is the **800×448 sub-stream** and `/stream1` the 1080p; one camera emits only **about 7 frames/s** in the captured clip. Frame sizes must therefore be read from the stream (`ffprobe`/`ffmpeg` output), not deduced from ONVIF.
- The `fps=` filter upstream of `hwdownload` limits the GPU→CPU copy cost; it is what makes the VAAPI chain worthwhile.

## Pitfalls found
- **`scale_vaapi` fails** on this host ("requested VAProfile is not supported": no video-processing VPP); the `hwdownload` + software scale chain works. Installing `libigfxcmrt7` had **no effect**. Remaining lead, untested: `intel-media-va-driver-non-free` (Debian `non-free` component, replaces `intel-media-va-driver`), which ships the non-free Intel kernels. Not adopted: the current chain costs 32 % of a core for 4 cameras and the Docker image only uses the free driver.
- A bench that throws `stderr` away can time a failing command: the first "VAAPI + scale" figures were wrong for that reason (corrected above).
- A copy (`-c copy`) of a Thingino sub-stream to a transport stream needs `-bsf:v h264_mp4toannexb` or a Matroska container.
