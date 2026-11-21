# Supported machines

What was tested, what was not, and how to add your own measurements. Nothing here is promised beyond what the "Status" column says.

| Machine | Status | Notes |
|---|---|---|
| Intel Core i5-6500 mini PC, 8 GB (reference) | **tested and measured** | 12 cameras in 1080p with hardware decoding (VAAPI): see [Measurements](measures.md) |
| amd64 mini PC, 16 GB (for example a one-amd8 class machine) | **not tested yet** | Same image as the reference. Expected to do at least as well; hardware decoding needs an Intel GPU with VAAPI |
| Raspberry Pi 5, 8 GB | **not tested yet** | arm64 image never built or run to date; see below |
| Other arm64 or amd64 Linux with Docker | untested | The installer does not refuse them; it warns on arm64 |

## Raspberry Pi 5: what to expect, what is unknown

Known from the design (not from measurements):

- **Operating system**: 64-bit Raspberry Pi OS (Debian 13 based) or Debian 13 arm64; Docker with the Compose plugin 2.24 or newer. A 32-bit system cannot run the images.
- **Storage**: use an **SSD** (USB 3 or NVMe), not an SD card. The database and the video recordings write continuously; an SD card wears out. The installer warns when `/` is on an SD card.
- **Video decoding**: the Intel hardware decoder (VAAPI) is not available on arm64; video is decoded by the processor, so the number of cameras it holds is lower than on the reference machine. The figure is unknown until measured.
- **Object detection** (YOLO on CPU): not measured on arm64. Keep it off at first, then enable it on one camera and watch the load.
- **First installation**: building the image on the Pi takes a long time (expect well over 40 minutes) and needs memory; prefer a published image ([ADR 0062](adr/0062-published-images-with-local-build-fallback.md)). Building on another arm64 machine and publishing is also possible.
- **Zigbee dongle**: works the same way (`/dev/ttyUSB0` or `/dev/ttyACM0`).
- **Power and cooling**: use the official power supply and a heat sink or fan; `vcgencmd get_throttled` (reported by the machine card) shows whether the Pi was ever throttled.

## Measure your machine

After `make prod-deploy` (or `./install.sh`) and a few idle minutes:

```bash
tools/machine-card.sh > card.md          # add --with-backup to time a backup too
```

The card lists the hardware, the architecture of the image, the memory and CPU of each service at rest, the interface latency (median and 95th percentile over 50 requests) and, on a Pi, temperature and throttling. It is read-only and contains no password. Add the cold start time (`make prod-down`, then `time make prod-up`) and the number of cameras you hold, then paste the result in [Measurements](measures.md) or share it in an issue: a result on a machine that is not listed is the most useful contribution to this table.

## Dropping what a small machine cannot carry

- Fewer cameras, or lower stream resolution on the camera side.
- `VIDEO_QUOTA_GB` and `IOT_QUOTA_GB` in `.env` bound the history kept on disk.
- Grafana and Prometheus can be stopped (`docker compose stop grafana prometheus`) if memory is short; the application keeps working, you lose the dashboards.
- The local assistant (an LLM) is a separate, optional program that needs far more than a Pi 5 offers; do not plan on it there.
