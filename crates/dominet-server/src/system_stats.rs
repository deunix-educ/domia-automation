// SPDX-License-Identifier: AGPL-3.0-or-later
//! Machine statistics (processor, memory, disk, temperature).
//!
//! A collector (`spawn`) measures every `SYSTEM_STATS_SECS` seconds with `sysinfo` and publishes the result as the
//! **last state** of the hub's `system` subject: subscribed tabs receive it live, a new subscriber receives it
//! right away, and `GET /api/system-stats/` re-reads the same message (no second cache).
//!
//! The conversion of the raw measurements into [`SystemStatsDto`] (`build`) and the choice of the temperature (`pick_temperature`)
//! are pure functions, tested without touching the machine.
//!
//! In a container, `/proc` describes the host (processor, memory) but the disk is the container's and the
//! temperature sensors are only visible if `/sys/class/thermal` is mounted: absent values stay
//! `null` rather than wrong.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use dominet_proto::{CpuStats, DiskStats, MemoryStats, SystemStatsDto, Topic, WsServerMsg};
use sysinfo::{Components, Disks, MINIMUM_CPU_UPDATE_INTERVAL, MemoryRefreshKind, System};
use tokio::task::JoinHandle;

use crate::hub::Hub;

const GIB: f64 = 1024.0 * 1024.0 * 1024.0;

/// Raw measurements at an instant, before conversion.
#[derive(Debug, Clone, PartialEq)]
pub struct Raw {
    pub cpu_percent: f32,
    pub cpu_count: usize,
    pub mem_total: u64,
    pub mem_available: u64,
    /// (total size, available space) of the disk mounted on `/`.
    pub disk: Option<(u64, u64)>,
    /// (label, degrees Celsius) of each sensor.
    pub sensors: Vec<(String, f32)>,
}

fn round1(x: f64) -> f64 {
    (x * 10.0).round() / 10.0
}

fn percent(used: u64, total: u64) -> f64 {
    if total == 0 {
        return 0.0;
    }
    #[allow(clippy::cast_precision_loss)]
    round1((used as f64 / total as f64 * 100.0).clamp(0.0, 100.0))
}

#[allow(clippy::cast_precision_loss)]
fn gib(bytes: u64) -> f64 {
    round1(bytes as f64 / GIB)
}

/// Sensor to display: the processor first (labels `coretemp`, `package`, `cpu`, `tctl`, `k10temp`, `soc`),
/// otherwise the hottest of all. Non-finite values or values outside `0 < t <= 150` are ignored (absent or
/// faulty sensors, which report 0 or absurd values).
#[must_use]
pub fn pick_temperature(sensors: &[(String, f32)]) -> Option<f64> {
    const CPU_WORDS: [&str; 6] = ["coretemp", "package", "cpu", "tctl", "k10temp", "soc"];
    let valid = |t: f32| t.is_finite() && t > 0.0 && t <= 150.0;
    let hottest = |mut it: Box<dyn Iterator<Item = f32>>| {
        it.by_ref()
            .fold(None::<f32>, |m, t| Some(m.map_or(t, |m| m.max(t))))
    };
    let cpu = hottest(Box::new(
        sensors
            .iter()
            .filter(|(label, t)| {
                let l = label.to_ascii_lowercase();
                valid(*t) && CPU_WORDS.iter().any(|w| l.contains(w))
            })
            .map(|(_, t)| *t),
    ));
    let any = || {
        hottest(Box::new(
            sensors.iter().filter(|(_, t)| valid(*t)).map(|(_, t)| *t),
        ))
    };
    cpu.or_else(any).map(|t| round1(f64::from(t)))
}

/// Converts raw measurements to the API format (sizes in GiB to one decimal, percentages to one decimal).
#[must_use]
pub fn build(raw: &Raw, updated_at: f64) -> SystemStatsDto {
    let mem_used = raw.mem_total.saturating_sub(raw.mem_available);
    SystemStatsDto {
        cpu: CpuStats {
            percent: if raw.cpu_percent.is_finite() {
                round1(f64::from(raw.cpu_percent).clamp(0.0, 100.0))
            } else {
                0.0
            },
            count: u32::try_from(raw.cpu_count).unwrap_or(u32::MAX).max(1),
        },
        memory: MemoryStats {
            percent: percent(mem_used, raw.mem_total),
            used_gb: gib(mem_used),
            total_gb: gib(raw.mem_total),
            available_gb: gib(raw.mem_available),
        },
        disk: raw
            .disk
            .filter(|(total, _)| *total > 0)
            .map(|(total, free)| DiskStats {
                percent: percent(total.saturating_sub(free), total),
                used_gb: gib(total.saturating_sub(free)),
                total_gb: gib(total),
                free_gb: gib(free),
            }),
        temperature: pick_temperature(&raw.sensors),
        updated_at,
    }
}

fn unix_now() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0.0, |d| d.as_secs_f64())
}

/// `sysinfo` instruments kept from one measurement to the next (the processor load is a difference between two
/// readings).
#[derive(Debug)]
pub struct Sampler {
    sys: System,
    /// Absent if the platform does not provide them: the disk and the temperature then stay `null`.
    disks: Option<Disks>,
    components: Option<Components>,
}

impl Sampler {
    /// # Errors
    /// Returns an error if the platform does not allow reading the processor and the memory.
    pub fn new() -> Result<Self, sysinfo::Error> {
        let mut sys = System::new()?;
        // First reading: serves as the starting point for the processor load.
        sys.refresh_cpu_usage();
        Ok(Self {
            sys,
            disks: Disks::new().ok(),
            components: Components::new().ok(),
        })
    }

    /// Takes the measurements. Blocking (reads of `/proc` and `/sys`): to call outside the async reactor.
    pub fn sample(&mut self) -> Raw {
        self.sys.refresh_cpu_usage();
        self.sys
            .refresh_memory_specifics(MemoryRefreshKind::nothing().with_ram());
        if let Some(d) = &mut self.disks {
            d.refresh(true);
        }
        if let Some(c) = &mut self.components {
            c.refresh(true);
        }
        let root = std::path::Path::new("/");
        Raw {
            cpu_percent: self.sys.global_cpu_usage(),
            cpu_count: self.sys.cpus().len(),
            mem_total: self.sys.total_memory(),
            mem_available: self.sys.available_memory(),
            disk: self
                .disks
                .as_ref()
                .and_then(|d| d.iter().find(|d| d.mount_point() == root))
                .map(|d| (d.total_space(), d.available_space())),
            sensors: self.components.as_ref().map_or_else(Vec::new, |c| {
                c.iter()
                    .filter_map(|c| c.temperature().map(|t| (c.label().to_string(), t)))
                    .collect()
            }),
        }
    }
}

/// Measures now (the processor is then measured over a short interval): useful for tests and diagnostics.
pub async fn sample_once() -> Option<SystemStatsDto> {
    tokio::task::spawn_blocking(|| {
        let mut sampler = Sampler::new().ok()?;
        std::thread::sleep(MINIMUM_CPU_UPDATE_INTERVAL);
        Some(build(&sampler.sample(), unix_now()))
    })
    .await
    .ok()
    .flatten()
}

/// Starts the collector: a first measurement as soon as the processor can be measured, then one every `every`.
#[allow(clippy::must_use_candidate)] // the task may stay detached
pub fn spawn(hub: Hub, every: Duration) -> JoinHandle<()> {
    tokio::spawn(async move {
        let mut sampler = match Sampler::new() {
            Ok(s) => Some(s),
            Err(e) => {
                tracing::error!(error = %e, "machine resources unreadable: gauges disabled");
                return;
            }
        };
        tokio::time::sleep(MINIMUM_CPU_UPDATE_INTERVAL).await;
        let mut tick = tokio::time::interval(every);
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            tick.tick().await;
            let Some(mut s) = sampler.take() else { break };
            match tokio::task::spawn_blocking(move || {
                let raw = s.sample();
                (s, raw)
            })
            .await
            {
                Ok((s, raw)) => {
                    sampler = Some(s);
                    hub.publish_retained(
                        &Topic::System,
                        WsServerMsg::SystemStats(build(&raw, unix_now())),
                    );
                }
                Err(e) => {
                    tracing::error!(error = %e, "resource measurement interrupted");
                    break;
                }
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const GB: u64 = 1024 * 1024 * 1024;

    fn raw() -> Raw {
        Raw {
            cpu_percent: 37.46,
            cpu_count: 4,
            mem_total: 8 * GB,
            mem_available: 6 * GB,
            disk: Some((200 * GB, 50 * GB)),
            sensors: vec![],
        }
    }

    #[test]
    fn values_are_converted_and_rounded_like_the_reference_api() {
        let s = build(&raw(), 12.5);
        assert!((s.cpu.percent - 37.5).abs() < 1e-9);
        assert_eq!(s.cpu.count, 4);
        assert!((s.memory.percent - 25.0).abs() < 1e-9);
        assert!((s.memory.used_gb - 2.0).abs() < 1e-9);
        assert!((s.memory.total_gb - 8.0).abs() < 1e-9);
        assert!((s.memory.available_gb - 6.0).abs() < 1e-9);
        let d = s.disk.unwrap();
        assert!((d.percent - 75.0).abs() < 1e-9);
        assert!((d.used_gb - 150.0).abs() < 1e-9);
        assert!((d.free_gb - 50.0).abs() < 1e-9);
        assert!(s.temperature.is_none());
        assert!((s.updated_at - 12.5).abs() < 1e-9);
    }

    #[test]
    fn degenerate_measures_never_produce_nan_or_out_of_range_percentages() {
        let mut r = raw();
        r.cpu_percent = 250.0;
        r.mem_total = 0;
        r.mem_available = 5;
        r.cpu_count = 0;
        r.disk = Some((0, 0));
        let s = build(&r, 0.0);
        assert!((s.cpu.percent - 100.0).abs() < 1e-9);
        assert_eq!(s.cpu.count, 1);
        assert!(s.memory.percent.abs() < 1e-9);
        assert!(
            s.disk.is_none(),
            "un disque de taille nulle est absent, pas a 0 %"
        );
        r.cpu_percent = f32::NAN;
        assert!(!build(&r, 0.0).cpu.percent.is_nan());
    }

    #[test]
    fn the_cpu_sensor_wins_over_hotter_ones() {
        let sensors = vec![
            ("nvme Composite".to_string(), 70.0),
            ("coretemp Package id 0".to_string(), 48.0),
            ("coretemp Core 1".to_string(), 51.5),
        ];
        assert_eq!(pick_temperature(&sensors), Some(51.5));
    }

    #[test]
    fn without_a_cpu_sensor_the_hottest_valid_one_is_used() {
        let sensors = vec![
            ("acpitz".to_string(), 40.0),
            ("nvme Composite".to_string(), 44.0),
            ("broken".to_string(), 0.0),
            ("absurd".to_string(), 999.0),
            ("nan".to_string(), f32::NAN),
        ];
        assert_eq!(pick_temperature(&sensors), Some(44.0));
    }

    #[test]
    fn no_valid_sensor_means_no_temperature() {
        assert_eq!(pick_temperature(&[]), None);
        assert_eq!(pick_temperature(&[("cpu".to_string(), 0.0)]), None);
    }

    #[tokio::test]
    async fn the_real_machine_gives_plausible_values() {
        let s = sample_once().await.expect("mesure");
        assert!((0.0..=100.0).contains(&s.cpu.percent));
        assert!(s.cpu.count >= 1);
        assert!(s.memory.total_gb > 0.0);
        assert!((0.0..=100.0).contains(&s.memory.percent));
        assert!(s.updated_at > 1_600_000_000.0);
    }

    #[tokio::test]
    async fn the_collector_publishes_the_last_state_on_the_system_topic() {
        let hub = Hub::new();
        let task = spawn(hub.clone(), Duration::from_millis(50));
        let mut got = None;
        for _ in 0..100 {
            if let Some(m) = hub.retained(&Topic::System) {
                got = Some(m);
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        task.abort();
        let m = got.expect("une mesure doit etre publiee");
        assert!(matches!(&*m, WsServerMsg::SystemStats(s) if s.memory.total_gb > 0.0));
    }
}
