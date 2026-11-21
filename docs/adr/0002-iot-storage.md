# 0002 — IoT storage: ReductStore (raw) + PostgreSQL (aggregates)

- Status: accepted
- Date: 2026-10-31
- Spike verdict: **go**

## Context
Raw IoT data goes to ReductStore (`iot` bucket); aggregates and recent state go to PostgreSQL. ReductStore is not a time-series database (no aggregation). A spike (`reduct-rs` 1.21.0 against server 1.21.5, on an Intel Core i5-6500) validated the label model, performance on **1 million records** and FIFO retention.

## Model tested
Bucket `iot`, one entry `dev_{device_id}`, one record per state change: body = JSON of the full state (about 100 bytes), labels = one per tracked numeric key (`temperature`, `humidity`, `battery`, `linkquality`), timestamp in µs. Data set: 100 devices × 10,000 points (1 point / 10 s).

## Measurements
| Measure | Result |
|---|---|
| Ingestion, 1 M records with 4 labels, 8 parallel streams, batches of 50 | 595 s = **1,680 records/s** |
| Single write (`write_record`) | 818 records/s (1.2 ms) |
| Storage | **232 bytes/record** (227 MB on disk for 1 M) |
| Last hour of a device (about 300 records, bodies read) | 11 ms |
| Full history of a device (10,000 records), headers only / bodies read | 367 ms / 1.1 s |
| `temperature > 28` on one device (labels, 10,000 → 2,312) | 99 ms |
| `temperature > 28 AND battery < 90` | 67 ms |
| `temperature > 29.5` over **all 100 entries** (1 M scanned → 100,394) | 3.7 s |
| Last value of a device (`read_record` without timestamp) | 0.6 ms |
| Hourly average over 24 h, computed client side (8,600 points) | 0.93 s |
| FIFO quota 10 MB, 30,000 writes of 1 KB | size bounded to 10.4 MB, oldest evicted **by blocks** |

Numeric comparisons (`$gt`, `$lt`, `$and`) work on labels stored as strings.

## Decision
- **`iot` bucket** as above; one label per numeric key of `tracked_keys`. **Single** write per message (the real rate of about 50–100 msg/s is far below 818/s); optional batches, capped at **50 records** (see pitfalls).
- **Roles**: ReductStore = raw archive, forensics and threshold queries; **PostgreSQL = everything that feeds charts and dashboards** (recent state, `device_state_log`, energy aggregates). An hourly aggregate computed from ReductStore (about 1 s per day per device) is too slow for an interactive display.
- **Retention**: FIFO quota per bucket, **configurable** (`AppSetting`), proposed default **5 GB** for `iot` (about 21 M records, months at the real rate since only changes of `tracked_keys` are written). `device_state_log` (PostgreSQL): retention in days, configurable, proposed default **30 days**; daily aggregates are kept without limit.
- `max_block_size` is left at the default for `iot`; set to 1 MB for the small test buckets.

## Consequences
- A multi-entry threshold query ("all probes > X") costs a few seconds at 1 M records: fine for investigation, not for a real-time page.
- The FIFO quota evicts by blocks, so the size oscillates around the quota (about ±1 block); real disk space needs a margin.
- Numeric keys absent from a message have no label: conditional queries ignore them.

## Pitfalls found
- **Batch size**: batch metadata travels in HTTP headers; beyond about 50–100 records the server answers `RequestHeaderFieldsTooLarge` (even without labels at 200). `write_record_batch` (v2) accepts records from several entries.
- **Asynchronous bucket deletion**: right after a `DELETE`, writes fail with "Bucket is being deleted"; retry.
- `BucketInfo` exposes no record count: get it by summing `EntryInfo.record_count`. `Bucket` is not `Clone` (share it through `Arc`).
- A non-empty `RS_API_TOKEN` is mandatory in 1.21; the image has neither `wget` nor `curl`.
- Not covered here: the `video`/`snapshots`/`detections` buckets (records of several MB, streamed reads).
