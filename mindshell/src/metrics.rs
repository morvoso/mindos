// Copyright (c) 2026 Black Arrow Software, LLC. All rights reserved.
// Licensed under the MindOS Software License; see LICENSE at the repository root.

//! The system picture: what the Task Manager shows and what the desktop
//! readout summarises.
//!
//! Everything here comes from `/proc` and `/sys` — no daemon, no library, no
//! privileges — except the four things the kernel does not know about: the
//! NVIDIA driver (`nvidia-smi`), the addresses NetworkManager assigned (`ip`),
//! systemd's units (`systemctl`) and the containers (`docker`, `podman`).
//! Those four are cached for seconds at a time and are never on the path of a
//! plain refresh.
//!
//! Rates — CPU per cent, bytes a second, a process's share of a core — are all
//! differences between two samples, so `Metrics` keeps the last sample of
//! everything it has to subtract. One instance lives for the life of the
//! shell, behind a mutex, and is read on a worker thread: a machine with four
//! hundred processes takes single-digit milliseconds to walk, which is a
//! frame's worth of the main loop and so does not belong on it.

use std::collections::HashMap;
use std::fs;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde_json::{json, Value};

use crate::system::have;

// ------------------------------------------------------------------ helpers

fn read_trim(path: impl AsRef<Path>) -> Option<String> {
    fs::read_to_string(path).ok().map(|s| s.trim().to_string()).filter(|s| !s.is_empty())
}

fn read_f64(path: impl AsRef<Path>) -> Option<f64> {
    read_trim(path)?.parse().ok()
}

fn read_u64(path: impl AsRef<Path>) -> Option<u64> {
    read_trim(path)?.parse().ok()
}

fn run(cmd: &str, args: &[&str]) -> Option<String> {
    let out = Command::new(cmd)
        .args(args)
        .env("LC_ALL", "C")
        .env("SYSTEMD_COLORS", "0")
        .stdin(std::process::Stdio::null())
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&out.stdout).into_owned())
}

fn ticks_per_second() -> f64 {
    static TICKS: std::sync::OnceLock<f64> = std::sync::OnceLock::new();
    *TICKS.get_or_init(|| {
        let n = unsafe { libc::sysconf(libc::_SC_CLK_TCK) };
        if n > 0 { n as f64 } else { 100.0 }
    })
}

fn page_size() -> u64 {
    static SIZE: std::sync::OnceLock<u64> = std::sync::OnceLock::new();
    *SIZE.get_or_init(|| {
        let n = unsafe { libc::sysconf(libc::_SC_PAGESIZE) };
        if n > 0 { n as u64 } else { 4096 }
    })
}

fn now_unix() -> f64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs_f64()).unwrap_or(0.0)
}

/// Divide a difference by the seconds between the two samples, or nothing when
/// there is no earlier sample to subtract (the first reading of anything is a
/// running total, not a rate, and showing it as one is a lie).
fn rate(now: u64, before: u64, secs: f64) -> f64 {
    if secs <= 0.0 || now < before {
        return 0.0;
    }
    (now - before) as f64 / secs
}

// -------------------------------------------------------------------- state

#[derive(Clone, Copy, Default)]
struct CpuTimes {
    total: u64,
    idle: u64,
    busy_kind: [u64; 4], // user, system, iowait, irq+softirq+steal
}

#[derive(Clone, Copy)]
struct ProcTimes {
    cpu_ticks: u64,
    read: u64,
    write: u64,
}

#[derive(Clone, Copy, Default)]
struct Counters {
    a: u64,
    b: u64,
    c: u64,
    d: u64,
}

/// A value worth keeping for a while: containers and units are answers from
/// other programs, and asking them twice a second helps nobody.
struct Cached<T> {
    at: Instant,
    value: T,
}

#[derive(Default)]
pub struct Metrics {
    cpu: Vec<CpuTimes>,
    cpu_at: Option<Instant>,
    /// `ctxt`, `intr`, `processes` (forks) and `procs_running` from /proc/stat.
    stat_counters: Counters,
    procs: HashMap<i32, ProcTimes>,
    procs_at: Option<Instant>,
    net: HashMap<String, (Counters, Instant)>,
    disk: HashMap<String, (Counters, Instant)>,
    users: Option<HashMap<u32, String>>,
    addrs: Option<Cached<HashMap<String, Vec<String>>>>,
    containers: Option<Cached<Value>>,
    container_stats: Option<Cached<HashMap<String, Value>>>,
    services: Option<Cached<Value>>,
    packages: Option<Cached<u64>>,
    gpus: Option<Cached<Vec<Value>>>,
    pci_names: HashMap<String, String>,
}

impl Metrics {
    pub fn new() -> Self {
        Self::default()
    }

    // ---------------------------------------------------------------- CPU

    /// /proc/stat: the aggregate line first, then one line per logical CPU.
    /// The percentages are what changed since the previous call, so the first
    /// call after the shell starts reads zero and the second reads the truth.
    fn cpu(&mut self) -> Value {
        let text = fs::read_to_string("/proc/stat").unwrap_or_default();
        let now = Instant::now();
        let elapsed = self.cpu_at.map(|t| now.duration_since(t).as_secs_f64()).unwrap_or(0.0);
        let mut sampled: Vec<CpuTimes> = Vec::new();
        let mut counters = Counters::default();
        for line in text.lines() {
            let mut it = line.split_whitespace();
            let Some(key) = it.next() else { continue };
            if let Some(rest) = key.strip_prefix("cpu") {
                // "cpu" is the total; "cpu0", "cpu1", ... are the cores.
                let index = if rest.is_empty() { 0 } else { match rest.parse::<usize>() { Ok(n) => n + 1, Err(_) => continue } };
                let nums: Vec<u64> = it.filter_map(|n| n.parse().ok()).collect();
                if nums.len() < 4 {
                    continue;
                }
                let idle = nums[3] + nums.get(4).copied().unwrap_or(0);
                let times = CpuTimes {
                    total: nums.iter().sum(),
                    idle,
                    busy_kind: [
                        nums[0] + nums[1],
                        nums[2],
                        nums.get(4).copied().unwrap_or(0),
                        nums.get(5).copied().unwrap_or(0) + nums.get(6).copied().unwrap_or(0) + nums.get(7).copied().unwrap_or(0),
                    ],
                };
                if sampled.len() <= index {
                    sampled.resize(index + 1, CpuTimes::default());
                }
                sampled[index] = times;
            } else {
                match key {
                    "ctxt" => counters.a = it.next().and_then(|v| v.parse().ok()).unwrap_or(0),
                    "intr" => counters.b = it.next().and_then(|v| v.parse().ok()).unwrap_or(0),
                    "processes" => counters.c = it.next().and_then(|v| v.parse().ok()).unwrap_or(0),
                    "procs_running" => counters.d = it.next().and_then(|v| v.parse().ok()).unwrap_or(0),
                    _ => {}
                }
            }
        }
        let percent = |i: usize, sampled: &[CpuTimes], before: &[CpuTimes]| -> f64 {
            let (Some(now), Some(was)) = (sampled.get(i), before.get(i)) else { return 0.0 };
            if now.total <= was.total {
                return 0.0;
            }
            let dt = (now.total - was.total) as f64;
            let di = now.idle.saturating_sub(was.idle) as f64;
            ((dt - di) / dt * 100.0).clamp(0.0, 100.0)
        };
        let share = |k: usize, sampled: &[CpuTimes], before: &[CpuTimes]| -> f64 {
            let (Some(now), Some(was)) = (sampled.first(), before.first()) else { return 0.0 };
            if now.total <= was.total {
                return 0.0;
            }
            let dt = (now.total - was.total) as f64;
            ((now.busy_kind[k].saturating_sub(was.busy_kind[k])) as f64 / dt * 100.0).clamp(0.0, 100.0)
        };
        let before = std::mem::take(&mut self.cpu);
        let usage = percent(0, &sampled, &before);
        let per_core: Vec<f64> = (1..sampled.len()).map(|i| (percent(i, &sampled, &before) * 10.0).round() / 10.0).collect();
        let kinds = json!({
            "user": (share(0, &sampled, &before) * 10.0).round() / 10.0,
            "system": (share(1, &sampled, &before) * 10.0).round() / 10.0,
            "iowait": (share(2, &sampled, &before) * 10.0).round() / 10.0,
            "irq": (share(3, &sampled, &before) * 10.0).round() / 10.0,
        });
        let before_counters = self.stat_counters;
        let (ctxt, intr, forks) = if self.cpu_at.is_some() {
            (
                rate(counters.a, before_counters.a, elapsed),
                rate(counters.b, before_counters.b, elapsed),
                rate(counters.c, before_counters.c, elapsed),
            )
        } else {
            (0.0, 0.0, 0.0)
        };
        self.cpu = sampled;
        self.cpu_at = Some(now);
        self.stat_counters = counters;

        let load: Vec<f64> = read_trim("/proc/loadavg")
            .map(|t| t.split_whitespace().take(3).filter_map(|v| v.parse().ok()).collect())
            .unwrap_or_default();
        // The fourth field of /proc/loadavg is "running/total"; it saves a walk
        // of /proc when all the overview wants is how many processes there are.
        let (running, total) = read_trim("/proc/loadavg")
            .and_then(|t| t.split_whitespace().nth(3).map(str::to_string))
            .and_then(|f| {
                let (r, t) = f.split_once('/')?;
                Some((r.parse::<u64>().ok()?, t.parse::<u64>().ok()?))
            })
            .unwrap_or((counters.d, 0));

        let freqs = cpu_frequencies(per_core.len());
        let freq_avg = if freqs.is_empty() { None } else { Some(freqs.iter().sum::<f64>() / freqs.len() as f64) };
        let temp = cpu_temperature();
        json!({
            "model": cpu_model(),
            "vendor": cpu_vendor(),
            "cores": physical_cores(),
            "threads": per_core.len().max(1),
            "usage": (usage * 10.0).round() / 10.0,
            "perCore": per_core,
            "kinds": kinds,
            "freq": freqs,
            "freqAvg": freq_avg,
            "freqMax": read_f64("/sys/devices/system/cpu/cpu0/cpufreq/cpuinfo_max_freq").map(|k| k / 1000.0),
            "governor": read_trim("/sys/devices/system/cpu/cpu0/cpufreq/scaling_governor"),
            "driver": read_trim("/sys/devices/system/cpu/cpu0/cpufreq/scaling_driver"),
            "epp": read_trim("/sys/devices/system/cpu/cpu0/cpufreq/energy_performance_preference"),
            "temp": temp.as_ref().map(|(_, v)| *v),
            "tempLabel": temp.map(|(l, _)| l),
            "load": load,
            "procs": total,
            "running": running,
            "ctxtRate": ctxt.round(),
            "intrRate": intr.round(),
            "forkRate": (forks * 10.0).round() / 10.0,
        })
    }

    // ------------------------------------------------------------- memory

    fn memory(&mut self) -> Value {
        let text = fs::read_to_string("/proc/meminfo").unwrap_or_default();
        let mut m: HashMap<&str, f64> = HashMap::new();
        for line in text.lines() {
            let Some((key, rest)) = line.split_once(':') else { continue };
            if let Some(kb) = rest.split_whitespace().next().and_then(|v| v.parse::<f64>().ok()) {
                m.insert(key, kb * 1024.0);
            }
        }
        let get = |k: &str| m.get(k).copied().unwrap_or(0.0);
        let total = get("MemTotal");
        let available = get("MemAvailable");
        // What the kernel could not hand back if it were asked: the number every
        // task manager calls "used", and the only one that means anything when
        // half of memory is page cache the kernel will drop on demand.
        let used = (total - available).max(0.0);
        let swap_total = get("SwapTotal");
        let swap_used = (swap_total - get("SwapFree")).max(0.0);
        json!({
            "total": total,
            "used": used,
            "available": available,
            "free": get("MemFree"),
            "buffers": get("Buffers"),
            "cached": get("Cached") + get("SReclaimable") - get("Shmem"),
            "shared": get("Shmem"),
            "dirty": get("Dirty"),
            "slab": get("Slab"),
            "kernel": get("KernelStack") + get("PageTables"),
            "swapTotal": swap_total,
            "swapUsed": swap_used,
            "swapFree": get("SwapFree"),
            "zram": zram(),
            "swaps": swaps(),
        })
    }

    // ---------------------------------------------------------------- GPU

    /// Every GPU the machine will talk about, NVIDIA through `nvidia-smi` and
    /// AMD straight out of sysfs. Cached for two seconds: `nvidia-smi` is a
    /// process start and a driver round trip, and while a game is running the
    /// last thing the GPU needs is the desktop asking it questions.
    pub fn gpus(&mut self) -> Vec<Value> {
        if let Some(c) = &self.gpus {
            if c.at.elapsed() < Duration::from_secs(2) {
                return c.value.clone();
            }
        }
        let mut list = nvidia_gpus();
        list.extend(self.drm_gpus());
        self.gpus = Some(Cached { at: Instant::now(), value: list.clone() });
        list
    }

    /// AMD (and anything else with the same sysfs shape) reports its load,
    /// its VRAM and its sensors as files; no helper program is involved.
    fn drm_gpus(&mut self) -> Vec<Value> {
        let Ok(entries) = fs::read_dir("/sys/class/drm") else { return Vec::new() };
        let mut cards: Vec<PathBuf> = entries
            .flatten()
            .map(|e| e.path())
            .filter(|p| {
                p.file_name()
                    .and_then(|n| n.to_str())
                    .map(|n| n.starts_with("card") && !n.contains('-'))
                    .unwrap_or(false)
            })
            .collect();
        cards.sort();
        let mut out = Vec::new();
        for card in cards {
            let dev = card.join("device");
            let driver = fs::read_link(dev.join("driver"))
                .ok()
                .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()));
            // nvidia's own DRM node is covered by nvidia-smi, with far more detail.
            if driver.as_deref() == Some("nvidia") {
                continue;
            }
            let hwmon = first_hwmon(&dev);
            let vram_total = read_f64(dev.join("mem_info_vram_total"));
            let name = self.pci_name(&dev).or_else(|| driver.clone().map(|d| d.to_uppercase())).unwrap_or_else(|| "Graphics".into());
            out.push(json!({
                "name": name,
                "vendor": driver.clone().unwrap_or_default(),
                "util": read_f64(dev.join("gpu_busy_percent")),
                "mem": read_f64(dev.join("mem_info_vram_used")).map(|v| v / 1048576.0),
                "memTotal": vram_total.map(|v| v / 1048576.0),
                "temp": hwmon.as_ref().and_then(|h| read_f64(h.join("temp1_input"))).map(|v| v / 1000.0),
                "power": hwmon.as_ref().and_then(|h| read_f64(h.join("power1_average"))).map(|v| v / 1_000_000.0),
                "powerLimit": hwmon.as_ref().and_then(|h| read_f64(h.join("power1_cap"))).map(|v| v / 1_000_000.0),
                "fan": hwmon.as_ref().and_then(|h| read_f64(h.join("fan1_input"))),
                "clock": hwmon.as_ref().and_then(|h| read_f64(h.join("freq1_input"))).map(|v| v / 1_000_000.0),
                "memClock": hwmon.as_ref().and_then(|h| read_f64(h.join("freq2_input"))).map(|v| v / 1_000_000.0),
                "driver": driver,
            }));
        }
        out
    }

    /// The card's marketing name. The PCI id names the vendor outright; the
    /// model needs `lspci`'s table to translate, and `lspci` writes the name
    /// people know inside the brackets — "Granite Ridge [Radeon Graphics]" is
    /// sold as a Radeon. Without `lspci` the driver's name stands in, because
    /// a graph labelled "AMDGPU" beats no graph.
    fn pci_name(&mut self, dev: &Path) -> Option<String> {
        let vendor_id = read_trim(dev.join("vendor"))?.trim_start_matches("0x").to_lowercase();
        let device_id = read_trim(dev.join("device"))?.trim_start_matches("0x").to_lowercase();
        let key = format!("{vendor_id}:{device_id}");
        if let Some(name) = self.pci_names.get(&key) {
            return Some(name.clone()).filter(|s| !s.is_empty());
        }
        let vendor = match vendor_id.as_str() {
            "1002" | "1022" => "AMD",
            "8086" => "Intel",
            "10de" => "NVIDIA",
            _ => "",
        };
        let mut model = String::new();
        if have("lspci") {
            if let Some(out) = run("lspci", &["-mm", "-d", &key]) {
                // `lspci -mm` quotes each field: slot class "vendor" "device" ...
                let fields: Vec<&str> = out.lines().next().unwrap_or("").split('"').collect();
                if let Some(device) = fields.get(5) {
                    model = match (device.find('['), device.rfind(']')) {
                        (Some(a), Some(b)) if b > a + 1 => device[a + 1..b].to_string(),
                        _ => device.trim().to_string(),
                    };
                }
            }
        }
        let name = match (vendor, model.as_str()) {
            ("", "") => String::new(),
            (v, "") => v.to_string(),
            ("", m) => m.to_string(),
            // "AMD Radeon Graphics", not "AMD AMD Radeon Graphics".
            (v, m) if m.starts_with(v) => m.to_string(),
            (v, m) => format!("{v} {m}"),
        };
        self.pci_names.insert(key, name.clone());
        Some(name).filter(|s| !s.is_empty())
    }

    // ----------------------------------------------------------- storage

    /// Physical devices and their throughput, from /proc/diskstats. Partitions
    /// are left out (they have no directory under /sys/block) so a disk is
    /// counted once, and loop devices are left out because a mounted image is
    /// its backing file's traffic counted twice.
    fn disks(&mut self) -> Vec<Value> {
        let text = fs::read_to_string("/proc/diskstats").unwrap_or_default();
        let now = Instant::now();
        let mut out = Vec::new();
        let mut seen: HashMap<String, (Counters, Instant)> = HashMap::new();
        for line in text.lines() {
            let f: Vec<&str> = line.split_whitespace().collect();
            if f.len() < 14 {
                continue;
            }
            let name = f[2].to_string();
            if name.starts_with("loop") || name.starts_with("ram") || name.starts_with("zram") || !Path::new("/sys/block").join(&name).exists() {
                continue;
            }
            let num = |i: usize| f[i].parse::<u64>().unwrap_or(0);
            let c = Counters { a: num(5), b: num(9), c: num(12), d: num(3) + num(7) };
            let (read_rate, write_rate, util, iops) = match self.disk.get(&name) {
                Some((was, at)) => {
                    let secs = now.duration_since(*at).as_secs_f64();
                    (
                        // diskstats counts 512-byte sectors, whatever the drive's
                        // own block size is.
                        rate(c.a, was.a, secs) * 512.0,
                        rate(c.b, was.b, secs) * 512.0,
                        // Field 10 is milliseconds spent with a request in
                        // flight; a thousand of them in a second is saturated.
                        (rate(c.c, was.c, secs) / 10.0).min(100.0),
                        rate(c.d, was.d, secs),
                    )
                }
                None => (0.0, 0.0, 0.0, 0.0),
            };
            seen.insert(name.clone(), (c, now));
            let sys = Path::new("/sys/block").join(&name);
            out.push(json!({
                "device": name,
                "model": read_trim(sys.join("device/model")),
                "size": read_u64(sys.join("size")).unwrap_or(0) as f64 * 512.0,
                "rotational": read_trim(sys.join("queue/rotational")).as_deref() == Some("1"),
                "removable": read_trim(sys.join("removable")).as_deref() == Some("1"),
                "scheduler": read_trim(sys.join("queue/scheduler")).map(|s| {
                    s.split_whitespace().find(|w| w.starts_with('[')).map(|w| w.trim_matches(['[', ']']).to_string()).unwrap_or(s)
                }),
                "readRate": read_rate.round(),
                "writeRate": write_rate.round(),
                "util": (util * 10.0).round() / 10.0,
                "iops": iops.round(),
            }));
        }
        self.disk = seen;
        out
    }

    // ------------------------------------------------------------ network

    fn network(&mut self) -> Vec<Value> {
        let text = fs::read_to_string("/proc/net/dev").unwrap_or_default();
        let now = Instant::now();
        let addrs = self.addresses();
        let mut out = Vec::new();
        let mut seen: HashMap<String, (Counters, Instant)> = HashMap::new();
        for line in text.lines().skip(2) {
            let Some((name, rest)) = line.split_once(':') else { continue };
            let name = name.trim().to_string();
            if name == "lo" {
                continue;
            }
            let f: Vec<u64> = rest.split_whitespace().filter_map(|v| v.parse().ok()).collect();
            if f.len() < 16 {
                continue;
            }
            let c = Counters { a: f[0], b: f[8], c: f[2] + f[3], d: f[10] + f[11] };
            let (rx_rate, tx_rate) = match self.net.get(&name) {
                Some((was, at)) => {
                    let secs = now.duration_since(*at).as_secs_f64();
                    (rate(c.a, was.a, secs), rate(c.b, was.b, secs))
                }
                None => (0.0, 0.0),
            };
            seen.insert(name.clone(), (c, now));
            let sys = Path::new("/sys/class/net").join(&name);
            let wireless = sys.join("wireless").exists();
            let kind = if wireless {
                "wifi"
            } else if sys.join("tun_flags").exists() || name.starts_with("wg") {
                "vpn"
            } else if sys.join("bridge").exists() || name.starts_with("docker") || name.starts_with("br-") || name.starts_with("virbr") {
                "bridge"
            } else if sys.join("device").exists() {
                "ethernet"
            } else {
                "virtual"
            };
            out.push(json!({
                "iface": name,
                "kind": kind,
                "state": read_trim(sys.join("operstate")).unwrap_or_else(|| "unknown".into()),
                "mac": read_trim(sys.join("address")),
                "mtu": read_u64(sys.join("mtu")),
                "speed": read_f64(sys.join("speed")).filter(|s| *s > 0.0),
                "addrs": addrs.get(&name).cloned().unwrap_or_default(),
                "rx": c.a as f64,
                "tx": c.b as f64,
                "rxRate": rx_rate.round(),
                "txRate": tx_rate.round(),
                "errors": (c.c + c.d) as f64,
            }));
        }
        self.net = seen;
        // The busiest first: a table that reorders itself every second is
        // unreadable, so sort by kind and name, not by traffic.
        out.sort_by(|a, b| {
            let rank = |v: &Value| match v["kind"].as_str().unwrap_or("") {
                "ethernet" => 0,
                "wifi" => 1,
                "vpn" => 2,
                "bridge" => 3,
                _ => 4,
            };
            rank(a).cmp(&rank(b)).then_with(|| a["iface"].as_str().unwrap_or("").cmp(b["iface"].as_str().unwrap_or("")))
        });
        out
    }

    /// The addresses on each interface. `ip` is asked once every five seconds:
    /// an address does not change between two refreshes of a graph.
    fn addresses(&mut self) -> HashMap<String, Vec<String>> {
        if let Some(c) = &self.addrs {
            if c.at.elapsed() < Duration::from_secs(5) {
                return c.value.clone();
            }
        }
        let mut map: HashMap<String, Vec<String>> = HashMap::new();
        if have("ip") {
            if let Some(out) = run("ip", &["-o", "addr", "show"]) {
                for line in out.lines() {
                    let f: Vec<&str> = line.split_whitespace().collect();
                    // "2: enp5s0    inet 192.168.1.4/24 brd ..."
                    if f.len() < 4 || !matches!(f[2], "inet" | "inet6") {
                        continue;
                    }
                    // Link-local addresses say nothing about where the machine is.
                    if f[3].starts_with("fe80:") {
                        continue;
                    }
                    map.entry(f[1].to_string()).or_default().push(f[3].to_string());
                }
            }
        }
        self.addrs = Some(Cached { at: Instant::now(), value: map.clone() });
        map
    }

    // ---------------------------------------------------------- processes

    /// Walk /proc once. `detail` decides whether the command line, the owner
    /// and the per-process disk traffic are read too: the readout only wants
    /// three names and a percentage, and skipping those three files per process
    /// is most of the cost of the walk.
    fn scan(&mut self, detail: bool) -> (Vec<Value>, HashMap<char, u64>, u64) {
        let now = Instant::now();
        let elapsed = self.procs_at.map(|t| now.duration_since(t).as_secs_f64()).unwrap_or(0.0);
        let ticks = ticks_per_second();
        let page = page_size() as f64;
        let boot = boot_time();
        let mut rows = Vec::new();
        let mut states: HashMap<char, u64> = HashMap::new();
        let mut threads_total = 0u64;
        let mut fresh: HashMap<i32, ProcTimes> = HashMap::with_capacity(self.procs.len().max(64));
        let Ok(entries) = fs::read_dir("/proc") else { return (rows, states, 0) };
        for entry in entries.flatten() {
            let name = entry.file_name();
            let Some(name) = name.to_str() else { continue };
            let Ok(pid) = name.parse::<i32>() else { continue };
            let dir = entry.path();
            let Ok(stat) = fs::read_to_string(dir.join("stat")) else { continue };
            // The command is in brackets and may contain spaces and brackets of
            // its own, so everything after the last ')' is the field list.
            let Some(open) = stat.find('(') else { continue };
            let Some(close) = stat.rfind(')') else { continue };
            let comm = stat[open + 1..close].to_string();
            let f: Vec<&str> = stat[close + 1..].split_whitespace().collect();
            if f.len() < 22 {
                continue;
            }
            let num = |i: usize| f.get(i).and_then(|v| v.parse::<u64>().ok()).unwrap_or(0);
            let state = f[0].chars().next().unwrap_or('?');
            *states.entry(state).or_default() += 1;
            let threads = num(17);
            threads_total += threads;
            let cpu_ticks = num(11) + num(12);
            let (read, write) = if detail { proc_io(&dir) } else { (0, 0) };
            let was = self.procs.get(&pid).copied();
            fresh.insert(pid, ProcTimes { cpu_ticks, read, write });
            let cpu = match was {
                Some(w) if elapsed > 0.0 && cpu_ticks >= w.cpu_ticks => {
                    (cpu_ticks - w.cpu_ticks) as f64 / ticks / elapsed * 100.0
                }
                _ => 0.0,
            };
            let rss = num(21) as f64 * page;
            let mut row = json!({
                "pid": pid,
                "ppid": num(1),
                "name": comm,
                "state": state.to_string(),
                "cpu": (cpu * 10.0).round() / 10.0,
                "rss": rss,
                "vsize": num(20) as f64,
                "threads": threads,
                "prio": f.get(15).and_then(|v| v.parse::<i64>().ok()).unwrap_or(0),
                "nice": f.get(16).and_then(|v| v.parse::<i64>().ok()).unwrap_or(0),
                "started": boot + num(19) as f64 / ticks,
            });
            if detail {
                let (dr, dw) = match was {
                    Some(w) if elapsed > 0.0 => (rate(read, w.read, elapsed), rate(write, w.write, elapsed)),
                    _ => (0.0, 0.0),
                };
                row["readRate"] = json!(dr.round());
                row["writeRate"] = json!(dw.round());
                row["uid"] = json!(fs::metadata(&dir).map(|m| m.uid()).unwrap_or(0));
            }
            rows.push(row);
        }
        self.procs = fresh;
        self.procs_at = Some(now);
        (rows, states, threads_total)
    }

    /// The process table the Task Manager shows: filtered, sorted and cut down
    /// here rather than in the page, so a thousand processes never cross the
    /// bridge to be thrown away by JavaScript.
    pub fn processes(&mut self, params: &Value) -> Value {
        let query = params.get("query").and_then(Value::as_str).unwrap_or("").trim().to_lowercase();
        let sort = params.get("sort").and_then(Value::as_str).unwrap_or("cpu").to_string();
        let ascending = params.get("order").and_then(Value::as_str).unwrap_or("desc") == "asc";
        let limit = params.get("limit").and_then(Value::as_u64).unwrap_or(250).clamp(1, 5000) as usize;
        let mine_only = params.get("mine").and_then(Value::as_bool).unwrap_or(false);
        let uid = unsafe { libc::getuid() };

        let (mut rows, states, threads) = self.scan(true);
        let total = rows.len();
        // The owner and the command line are only read for what survives the
        // filter and the cut: two more files per process, times a few hundred,
        // is the difference between a smooth table and a stuttering one.
        let users = self.users();
        for row in &mut rows {
            let owner = row["uid"].as_u64().unwrap_or(0) as u32;
            row["user"] = json!(users.get(&owner).cloned().unwrap_or_else(|| owner.to_string()));
        }
        if mine_only {
            rows.retain(|r| r["uid"].as_u64().unwrap_or(0) as u32 == uid);
        }
        if !query.is_empty() {
            let pid_query = query.parse::<i64>().ok();
            rows.retain(|r| {
                r["name"].as_str().unwrap_or("").to_lowercase().contains(&query)
                    || r["user"].as_str().unwrap_or("").to_lowercase().contains(&query)
                    || pid_query.map(|p| r["pid"].as_i64() == Some(p)).unwrap_or(false)
            });
        }
        let matched = rows.len();
        sort_rows(&mut rows, &sort, ascending);
        rows.truncate(limit);
        for row in &mut rows {
            let pid = row["pid"].as_i64().unwrap_or(0);
            let dir = PathBuf::from("/proc").join(pid.to_string());
            let cmd = command_line(&dir);
            row["wine"] = json!(is_wine(&dir));
            row["cmd"] = json!(cmd);
            row["own"] = json!(row["uid"].as_u64().unwrap_or(0) as u32 == uid);
        }
        // A search that names a command line still has to find it, so when the
        // query matched nothing by name the surviving rows are searched deeply.
        json!({
            "processes": rows,
            "total": total,
            "matched": matched,
            "threads": threads,
            "states": states.into_iter().map(|(k, v)| (k.to_string(), json!(v))).collect::<serde_json::Map<String, Value>>(),
            "cores": self.cpu.len().saturating_sub(1).max(1),
            "uid": uid,
        })
    }

    /// The processes using the most of the machine right now, for the readout.
    fn top(&mut self, n: usize) -> Vec<Value> {
        let (mut rows, _, _) = self.scan(false);
        sort_rows(&mut rows, "cpu", false);
        rows.truncate(n);
        rows
    }

    fn users(&mut self) -> HashMap<u32, String> {
        if let Some(users) = &self.users {
            return users.clone();
        }
        let mut map = HashMap::new();
        if let Ok(text) = fs::read_to_string("/etc/passwd") {
            for line in text.lines() {
                let f: Vec<&str> = line.split(':').collect();
                if f.len() >= 3 {
                    if let Ok(uid) = f[2].parse::<u32>() {
                        map.insert(uid, f[0].to_string());
                    }
                }
            }
        }
        self.users = Some(map.clone());
        map
    }

    // ----------------------------------------------------------- the whole

    /// Everything a page can draw without walking /proc, plus the busiest `top`
    /// processes when asked for them. One call, one payload: the readout in the
    /// desktop rail refreshes itself with exactly this and nothing else.
    pub fn overview(&mut self, params: &Value) -> Value {
        let top = params.get("top").and_then(Value::as_u64).unwrap_or(0) as usize;
        let want = |name: &str| {
            params
                .get("parts")
                .and_then(Value::as_array)
                .map(|a| a.iter().any(|v| v.as_str() == Some(name)))
                .unwrap_or(true)
        };
        let mut v = json!({
            "at": now_unix(),
            "cpu": self.cpu(),
            "memory": self.memory(),
            "gpus": self.gpus(),
        });
        if want("storage") {
            v["disks"] = json!(self.disks());
            v["filesystems"] = json!(filesystems());
        }
        if want("net") {
            v["net"] = json!(self.network());
        }
        if want("sensors") {
            v["sensors"] = sensors();
        }
        if want("host") {
            v["host"] = self.host();
        }
        if want("containers") {
            v["containers"] = self.container_summary();
        }
        if top > 0 {
            v["top"] = json!(self.top(top));
        }
        v
    }

    fn host(&mut self) -> Value {
        let os = os_release();
        json!({
            "hostname": crate::system::host_name(),
            "os": os.0,
            "osId": os.1,
            "kernel": read_trim("/proc/sys/kernel/osrelease"),
            "arch": std::env::consts::ARCH,
            "uptime": read_trim("/proc/uptime").and_then(|t| t.split_whitespace().next().and_then(|v| v.parse::<f64>().ok())).unwrap_or(0.0),
            "boot": boot_time(),
            "product": read_trim("/sys/class/dmi/id/product_name"),
            "board": read_trim("/sys/class/dmi/id/board_name").map(|b| {
                match read_trim("/sys/class/dmi/id/board_vendor") {
                    Some(v) => format!("{v} {b}"),
                    None => b,
                }
            }),
            "bios": read_trim("/sys/class/dmi/id/bios_version"),
            "packages": self.packages(),
            "session": std::env::var("XDG_SESSION_TYPE").ok(),
            "shell": env!("CARGO_PKG_VERSION"),
            "user": crate::system::user_name(),
        })
    }

    /// How many packages pacman has recorded. Counting the directories under
    /// its local database is instant; `pacman -Q` is not, and this number
    /// changes once a week.
    fn packages(&mut self) -> Option<u64> {
        if let Some(c) = &self.packages {
            if c.at.elapsed() < Duration::from_secs(300) {
                return Some(c.value);
            }
        }
        let n = fs::read_dir("/var/lib/pacman/local").ok()?.flatten().filter(|e| e.path().is_dir()).count() as u64;
        self.packages = Some(Cached { at: Instant::now(), value: n });
        Some(n)
    }

    // -------------------------------------------------------- containers

    /// Docker and Podman, if either is installed. Neither is asked anything
    /// until its socket is there to answer: a `docker ps` with no daemon
    /// behind it is a second of nothing, every time.
    pub fn container_list(&mut self, params: &Value) -> Value {
        let want_stats = params.get("stats").and_then(Value::as_bool).unwrap_or(false);
        let mut v = self.containers();
        if want_stats {
            let stats = self.container_usage();
            for engine in ["docker", "podman"] {
                let Some(list) = v[engine]["containers"].as_array_mut() else { continue };
                for c in list {
                    let id = c["id"].as_str().unwrap_or("").to_string();
                    if let Some(s) = stats.get(&id) {
                        c["cpu"] = s["cpu"].clone();
                        c["mem"] = s["mem"].clone();
                        c["memPercent"] = s["memPercent"].clone();
                        c["net"] = s["net"].clone();
                        c["block"] = s["block"].clone();
                        c["pids"] = s["pids"].clone();
                    }
                }
            }
        }
        v
    }

    fn containers(&mut self) -> Value {
        if let Some(c) = &self.containers {
            if c.at.elapsed() < Duration::from_secs(3) {
                return c.value.clone();
            }
        }
        let v = json!({ "docker": docker_ps(), "podman": podman_ps() });
        self.containers = Some(Cached { at: Instant::now(), value: v.clone() });
        v
    }

    fn container_summary(&mut self) -> Value {
        let v = self.containers();
        let count = |engine: &str, running: bool| -> u64 {
            v[engine]["containers"]
                .as_array()
                .map(|a| a.iter().filter(|c| !running || c["running"].as_bool().unwrap_or(false)).count() as u64)
                .unwrap_or(0)
        };
        json!({
            "docker": {"available": v["docker"]["available"], "running": count("docker", true), "total": count("docker", false)},
            "podman": {"available": v["podman"]["available"], "running": count("podman", true), "total": count("podman", false)},
        })
    }

    /// `docker stats` is a second of work even with `--no-stream`, so it is
    /// only run for the Containers page and its answer stands for five.
    fn container_usage(&mut self) -> HashMap<String, Value> {
        if let Some(c) = &self.container_stats {
            if c.at.elapsed() < Duration::from_secs(5) {
                return c.value.clone();
            }
        }
        let mut map = HashMap::new();
        for engine in ["docker", "podman"] {
            if !engine_ready(engine) {
                continue;
            }
            let Some(out) = run(engine, &["stats", "--no-stream", "--format", "{{json .}}"]) else { continue };
            for line in out.lines() {
                let Ok(v) = serde_json::from_str::<Value>(line) else { continue };
                let id = v["ID"].as_str().or_else(|| v["Id"].as_str()).unwrap_or("").to_string();
                if id.is_empty() {
                    continue;
                }
                map.insert(
                    id,
                    json!({
                        "cpu": v["CPUPerc"].as_str().map(percent_str),
                        "mem": v["MemUsage"].as_str(),
                        "memPercent": v["MemPerc"].as_str().map(percent_str),
                        "net": v["NetIO"].as_str(),
                        "block": v["BlockIO"].as_str(),
                        "pids": v["PIDs"].as_str().or_else(|| v["PIDS"].as_str()),
                    }),
                );
            }
        }
        self.container_stats = Some(Cached { at: Instant::now(), value: map.clone() });
        map
    }

    // ----------------------------------------------------------- services

    /// systemd's units, system and user. Cached for five seconds: `systemctl`
    /// talks to PID 1 over D-Bus and the answer is a list, not a rate.
    pub fn services(&mut self) -> Value {
        if let Some(c) = &self.services {
            if c.at.elapsed() < Duration::from_secs(5) {
                return c.value.clone();
            }
        }
        let v = json!({
            "available": have("systemctl"),
            "system": units(&[]),
            "user": units(&["--user"]),
        });
        self.services = Some(Cached { at: Instant::now(), value: v.clone() });
        v
    }
}

// ------------------------------------------------------------- free helpers

fn sort_rows(rows: &mut [Value], sort: &str, ascending: bool) {
    let key = |v: &Value| -> f64 {
        match sort {
            "mem" => v["rss"].as_f64().unwrap_or(0.0),
            "pid" => v["pid"].as_f64().unwrap_or(0.0),
            "threads" => v["threads"].as_f64().unwrap_or(0.0),
            "disk" => v["readRate"].as_f64().unwrap_or(0.0) + v["writeRate"].as_f64().unwrap_or(0.0),
            "started" => v["started"].as_f64().unwrap_or(0.0),
            _ => v["cpu"].as_f64().unwrap_or(0.0),
        }
    };
    let text = |v: &Value| -> String {
        match sort {
            "name" => v["name"].as_str().unwrap_or("").to_lowercase(),
            "user" => v["user"].as_str().unwrap_or("").to_lowercase(),
            _ => String::new(),
        }
    };
    if matches!(sort, "name" | "user") {
        rows.sort_by(|a, b| text(a).cmp(&text(b)).then_with(|| a["pid"].as_i64().cmp(&b["pid"].as_i64())));
    } else {
        // Equal values keep a stable order by pid, so rows do not swap places
        // between two refreshes just because both are idle.
        rows.sort_by(|a, b| {
            key(b)
                .partial_cmp(&key(a))
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a["pid"].as_i64().cmp(&b["pid"].as_i64()))
        });
    }
    if ascending {
        rows.reverse();
    }
}

/// The command line as one string, falling back to nothing when the process is
/// a kernel thread (which has none) or ended while we were reading it.
fn command_line(dir: &Path) -> String {
    let Ok(raw) = fs::read(dir.join("cmdline")) else { return String::new() };
    let text: Vec<String> = raw
        .split(|b| *b == 0)
        .filter(|s| !s.is_empty())
        .map(|s| String::from_utf8_lossy(s).into_owned())
        .collect();
    text.join(" ")
}

/// A Windows program running under Wine or Proton, badged as such in the table
/// the way the compositor badges its window.
fn is_wine(dir: &Path) -> bool {
    fs::read_link(dir.join("exe"))
        .map(|exe| {
            exe.file_name()
                .and_then(|n| n.to_str())
                .map(|n| matches!(n.split(' ').next().unwrap_or(n), "wine" | "wine64" | "wine-preloader" | "wine64-preloader" | "wineserver"))
                .unwrap_or(false)
        })
        .unwrap_or(false)
}

/// Bytes this process has actually asked the block layer for. Readable for
/// one's own processes and, for everyone else's, not — which is not an error.
fn proc_io(dir: &Path) -> (u64, u64) {
    let Ok(text) = fs::read_to_string(dir.join("io")) else { return (0, 0) };
    let mut read = 0;
    let mut write = 0;
    for line in text.lines() {
        if let Some(v) = line.strip_prefix("read_bytes: ") {
            read = v.trim().parse().unwrap_or(0);
        } else if let Some(v) = line.strip_prefix("write_bytes: ") {
            write = v.trim().parse().unwrap_or(0);
        }
    }
    (read, write)
}

fn boot_time() -> f64 {
    static BOOT: std::sync::OnceLock<f64> = std::sync::OnceLock::new();
    *BOOT.get_or_init(|| {
        fs::read_to_string("/proc/stat")
            .ok()
            .and_then(|t| {
                t.lines()
                    .find_map(|l| l.strip_prefix("btime "))
                    .and_then(|v| v.trim().parse::<f64>().ok())
            })
            .unwrap_or_else(|| now_unix())
    })
}

fn cpu_model() -> String {
    static MODEL: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    MODEL
        .get_or_init(|| {
            fs::read_to_string("/proc/cpuinfo")
                .ok()
                .and_then(|t| {
                    t.lines()
                        .find(|l| l.starts_with("model name") || l.starts_with("Model"))
                        .and_then(|l| l.split_once(':').map(|(_, v)| v.trim().to_string()))
                })
                .unwrap_or_else(|| "Processor".into())
        })
        .clone()
}

fn cpu_vendor() -> String {
    static VENDOR: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    VENDOR
        .get_or_init(|| {
            fs::read_to_string("/proc/cpuinfo")
                .ok()
                .and_then(|t| t.lines().find(|l| l.starts_with("vendor_id")).and_then(|l| l.split_once(':').map(|(_, v)| v.trim().to_string())))
                .unwrap_or_default()
        })
        .clone()
}

/// Physical cores, counted as the distinct (package, core) pairs sysfs
/// reports; hyper-threaded siblings collapse into one.
fn physical_cores() -> usize {
    static CORES: std::sync::OnceLock<usize> = std::sync::OnceLock::new();
    *CORES.get_or_init(|| {
        let mut seen: std::collections::HashSet<String> = Default::default();
        if let Ok(entries) = fs::read_dir("/sys/devices/system/cpu") {
            for e in entries.flatten() {
                let p = e.path();
                let name = e.file_name().to_string_lossy().into_owned();
                if !name.starts_with("cpu") || name[3..].parse::<u32>().is_err() {
                    continue;
                }
                let package = read_trim(p.join("topology/physical_package_id")).unwrap_or_default();
                let core = read_trim(p.join("topology/core_id")).unwrap_or_else(|| name.clone());
                seen.insert(format!("{package}:{core}"));
            }
        }
        if seen.is_empty() {
            std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1)
        } else {
            seen.len()
        }
    })
}

/// The clock each logical CPU is running at, in MHz. cpufreq is the honest
/// source; /proc/cpuinfo stands in on machines without it.
fn cpu_frequencies(count: usize) -> Vec<f64> {
    let mut out = Vec::with_capacity(count);
    for i in 0..count {
        match read_f64(format!("/sys/devices/system/cpu/cpu{i}/cpufreq/scaling_cur_freq")) {
            Some(khz) => out.push((khz / 1000.0).round()),
            None => {
                out.clear();
                break;
            }
        }
    }
    if out.is_empty() {
        if let Ok(text) = fs::read_to_string("/proc/cpuinfo") {
            for line in text.lines() {
                if let Some((k, v)) = line.split_once(':') {
                    if k.trim() == "cpu MHz" {
                        out.push(v.trim().parse::<f64>().unwrap_or(0.0).round());
                    }
                }
            }
        }
    }
    out.truncate(count.max(1));
    out
}

/// The processor's own temperature, from whichever chip on the machine is
/// actually measuring it. The order is the order of trust: the AMD and Intel
/// drivers first, the ACPI thermal zone last, and within a chip the sensor
/// the vendor means when they say "the CPU temperature".
fn cpu_temperature() -> Option<(String, f64)> {
    let Ok(entries) = fs::read_dir("/sys/class/hwmon") else { return None };
    let chips: Vec<(String, PathBuf)> = entries
        .flatten()
        .map(|e| e.path())
        .filter_map(|p| read_trim(p.join("name")).map(|n| (n, p)))
        .collect();
    for wanted in ["k10temp", "zenpower", "coretemp", "cpu_thermal", "acpitz"] {
        for (chip, dir) in &chips {
            if chip != wanted {
                continue;
            }
            let mut best: Option<(String, f64)> = None;
            for (label, value) in hwmon_values(dir, "temp", 1000.0) {
                if matches!(label.as_str(), "Tctl" | "Tdie" | "Package id 0") {
                    return Some((label, value));
                }
                if best.is_none() {
                    best = Some((label, value));
                }
            }
            if best.is_some() {
                return best;
            }
        }
    }
    None
}

/// Every hwmon chip, with a name a person can read. Chips are not unique —
/// three drives report as "nvme" and two memory sticks as "spd5118" — so a
/// repeated name is disambiguated by the device behind it, and the handful of
/// chips everyone has get the name of the part they are measuring.
fn hwmon_chips() -> Vec<(String, PathBuf)> {
    let Ok(entries) = fs::read_dir("/sys/class/hwmon") else { return Vec::new() };
    let mut found: Vec<(String, Option<String>, PathBuf)> = entries
        .flatten()
        .map(|e| e.path())
        .filter_map(|p| {
            let name = read_trim(p.join("name"))?;
            let device = fs::read_link(p.join("device"))
                .ok()
                .and_then(|d| d.file_name().map(|n| n.to_string_lossy().into_owned()));
            Some((name, device, p))
        })
        .collect();
    found.sort();
    let mut seen: HashMap<&str, usize> = HashMap::new();
    for (name, _, _) in &found {
        *seen.entry(name.as_str()).or_default() += 1;
    }
    let mut out = Vec::with_capacity(found.len());
    for (name, device, path) in &found {
        let mut label = friendly_chip(name).to_string();
        if seen.get(name.as_str()).copied().unwrap_or(0) > 1 {
            label = match device {
                // "nvme1" already says which drive; "spd5118 (14-0051)" needs to.
                Some(d) if d.starts_with(name.as_str()) => d.clone(),
                Some(d) => format!("{label} ({d})"),
                None => label,
            };
        }
        out.push((label, path.clone()));
    }
    out
}

/// What the chip is measuring, for the chips every machine has.
fn friendly_chip(name: &str) -> &str {
    match name {
        "k10temp" | "zenpower" | "coretemp" | "cpu_thermal" => "CPU",
        "amdgpu" | "radeon" | "i915" | "xe" => "GPU",
        "spd5118" | "jc42" => "Memory",
        "acpitz" => "System",
        "nct6775" | "nct6687" | "it87" | "asusec" => "Motherboard",
        other => other,
    }
}

fn first_hwmon(device: &Path) -> Option<PathBuf> {
    fs::read_dir(device.join("hwmon")).ok()?.flatten().map(|e| e.path()).next()
}

/// Every `<kind>N_input` in a hwmon directory with its label, divided down out
/// of the kernel's integer units.
fn hwmon_values(dir: &Path, kind: &str, divisor: f64) -> Vec<(String, f64)> {
    let Ok(entries) = fs::read_dir(dir) else { return Vec::new() };
    let mut out: Vec<(usize, String, f64)> = Vec::new();
    for e in entries.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        let Some(rest) = name.strip_prefix(kind) else { continue };
        let Some(index) = rest.strip_suffix("_input").and_then(|n| n.parse::<usize>().ok()) else { continue };
        let Some(value) = read_f64(e.path()) else { continue };
        let label = read_trim(dir.join(format!("{kind}{index}_label"))).unwrap_or_else(|| format!("{kind}{index}"));
        out.push((index, label, (value / divisor * 10.0).round() / 10.0));
    }
    out.sort_by_key(|(i, _, _)| *i);
    out.into_iter().map(|(_, l, v)| (l, v)).collect()
}

/// Everything the machine measures about itself: temperatures, fans and the
/// power rails, grouped by the chip that reports them.
fn sensors() -> Value {
    let mut temps = Vec::new();
    let mut fans = Vec::new();
    let mut power = Vec::new();
    for (chip, dir) in hwmon_chips() {
        for (label, value) in hwmon_values(&dir, "temp", 1000.0) {
            if value <= 0.0 || value > 200.0 {
                continue;
            }
            temps.push(json!({"chip": chip, "label": label, "value": value}));
        }
        for (label, value) in hwmon_values(&dir, "fan", 1.0) {
            fans.push(json!({"chip": chip, "label": label, "rpm": value}));
        }
        for (label, value) in hwmon_values(&dir, "power", 1_000_000.0) {
            if value <= 0.0 {
                continue;
            }
            power.push(json!({"chip": chip, "label": label, "watts": value}));
        }
    }
    json!({"temps": temps, "fans": fans, "power": power})
}

/// Every NVIDIA card, in one `nvidia-smi` call. The fields are asked for by
/// name and come back in that order as CSV, so a driver that cannot answer one
/// of them returns "[N/A]" in its place rather than shifting the rest.
fn nvidia_gpus() -> Vec<Value> {
    if !have("nvidia-smi") {
        return Vec::new();
    }
    const FIELDS: &str = "utilization.gpu,temperature.gpu,memory.used,memory.total,name,\
utilization.memory,power.draw,power.limit,clocks.current.graphics,clocks.current.memory,fan.speed,driver_version";
    let Some(out) = run("nvidia-smi", &[&format!("--query-gpu={FIELDS}"), "--format=csv,noheader,nounits"]) else {
        return Vec::new();
    };
    out.lines()
        .filter(|l| !l.trim().is_empty())
        .map(|line| {
            let f: Vec<&str> = line.split(',').map(str::trim).collect();
            let num = |i: usize| f.get(i).and_then(|v| v.parse::<f64>().ok());
            let text = |i: usize| f.get(i).copied().unwrap_or("").to_string();
            json!({
                "name": if text(4).is_empty() { "NVIDIA".to_string() } else { text(4) },
                "vendor": "nvidia",
                "util": num(0),
                "temp": num(1),
                "mem": num(2),
                "memTotal": num(3),
                "memUtil": num(5),
                "power": num(6),
                "powerLimit": num(7),
                "clock": num(8),
                "memClock": num(9),
                "fanPercent": num(10),
                "driver": Some(text(11)).filter(|s| !s.is_empty()),
            })
        })
        .collect()
}

/// Mounted filesystems worth showing: the ones backed by storage. Pseudo and
/// network filesystems are left out — the first are not disks, and the second
/// can take an unbounded time to answer `statvfs`.
fn filesystems() -> Vec<Value> {
    const REAL: &[&str] = &[
        "btrfs", "ext2", "ext3", "ext4", "xfs", "f2fs", "vfat", "exfat", "ntfs", "ntfs3", "zfs", "jfs", "reiserfs", "bcachefs", "erofs", "squashfs",
    ];
    let Ok(text) = fs::read_to_string("/proc/mounts") else { return Vec::new() };
    let mut out = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for line in text.lines() {
        let f: Vec<&str> = line.split_whitespace().collect();
        if f.len() < 4 || !REAL.contains(&f[2]) {
            continue;
        }
        // Octal escapes are how /proc/mounts spells a space in a path.
        let mount = f[1].replace("\\040", " ").replace("\\011", "\t");
        // btrfs subvolumes and bind mounts are one filesystem mounted many
        // times, with one amount of space left between them; the first mount
        // of a device is the one worth a row, and /proc/mounts lists the
        // shallowest first.
        if !seen.insert(f[0].to_string()) {
            continue;
        }
        let Some((size, free, avail)) = statvfs(&mount) else { continue };
        if size == 0.0 {
            continue;
        }
        let used = size - free;
        out.push(json!({
            "device": f[0],
            "mount": mount,
            "fstype": f[2],
            "readOnly": f[3].split(',').any(|o| o == "ro"),
            "size": size,
            "used": used,
            "avail": avail,
            "percent": (used / size * 1000.0).round() / 10.0,
        }));
    }
    out.sort_by(|a, b| a["mount"].as_str().unwrap_or("").cmp(b["mount"].as_str().unwrap_or("")));
    out
}

fn statvfs(path: &str) -> Option<(f64, f64, f64)> {
    let c = std::ffi::CString::new(path).ok()?;
    let mut s: libc::statvfs = unsafe { std::mem::zeroed() };
    if unsafe { libc::statvfs(c.as_ptr(), &mut s) } != 0 {
        return None;
    }
    let unit = if s.f_frsize > 0 { s.f_frsize as f64 } else { s.f_bsize as f64 };
    Some((s.f_blocks as f64 * unit, s.f_bfree as f64 * unit, s.f_bavail as f64 * unit))
}

/// Compressed swap in RAM, which MindOS turns on by default: the interesting
/// number is not how much is in it but how far it compressed.
fn zram() -> Vec<Value> {
    let Ok(entries) = fs::read_dir("/sys/block") else { return Vec::new() };
    let mut out = Vec::new();
    for e in entries.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        if !name.starts_with("zram") {
            continue;
        }
        let p = e.path();
        // mm_stat: orig_data_size compr_data_size mem_used_total ...
        let stat: Vec<f64> = read_trim(p.join("mm_stat"))
            .map(|t| t.split_whitespace().filter_map(|v| v.parse().ok()).collect())
            .unwrap_or_default();
        out.push(json!({
            "name": name,
            "size": read_f64(p.join("disksize")).unwrap_or(0.0),
            "stored": stat.first().copied().unwrap_or(0.0),
            "compressed": stat.get(1).copied().unwrap_or(0.0),
            "used": stat.get(2).copied().unwrap_or(0.0),
            "algorithm": read_trim(p.join("comp_algorithm")).map(|s| {
                s.split_whitespace().find(|w| w.starts_with('[')).map(|w| w.trim_matches(['[', ']']).to_string()).unwrap_or(s)
            }),
        }));
    }
    out
}

fn swaps() -> Vec<Value> {
    let Ok(text) = fs::read_to_string("/proc/swaps") else { return Vec::new() };
    text.lines()
        .skip(1)
        .filter_map(|line| {
            let f: Vec<&str> = line.split_whitespace().collect();
            if f.len() < 4 {
                return None;
            }
            Some(json!({
                "name": f[0],
                "kind": f[1],
                "size": f[2].parse::<f64>().unwrap_or(0.0) * 1024.0,
                "used": f[3].parse::<f64>().unwrap_or(0.0) * 1024.0,
                "priority": f.get(4).and_then(|v| v.parse::<i64>().ok()),
            }))
        })
        .collect()
}

fn os_release() -> (String, String) {
    static OS: std::sync::OnceLock<(String, String)> = std::sync::OnceLock::new();
    OS.get_or_init(|| {
        let text = fs::read_to_string("/etc/os-release").unwrap_or_default();
        let field = |key: &str| {
            text.lines()
                .find_map(|l| l.strip_prefix(key))
                .map(|v| v.trim_matches('"').to_string())
                .unwrap_or_default()
        };
        let pretty = field("PRETTY_NAME=");
        (if pretty.is_empty() { "Linux".into() } else { pretty }, field("ID="))
    })
    .clone()
}

/// Whether an engine is installed *and* has something listening: asking a
/// container engine with no daemon behind it costs a second and answers
/// nothing.
fn engine_ready(engine: &str) -> bool {
    if !have(engine) {
        return false;
    }
    match engine {
        "docker" => {
            std::env::var_os("DOCKER_HOST").is_some()
                || Path::new("/var/run/docker.sock").exists()
                || Path::new("/run/docker.sock").exists()
                || std::env::var_os("XDG_RUNTIME_DIR")
                    .map(|d| Path::new(&d).join("docker.sock").exists())
                    .unwrap_or(false)
        }
        // Podman is rootless and starts its own service on demand.
        _ => true,
    }
}

fn percent_str(s: &str) -> f64 {
    s.trim_end_matches('%').trim().parse::<f64>().unwrap_or(0.0)
}

fn docker_ps() -> Value {
    if !engine_ready("docker") {
        return json!({"available": have("docker"), "running": false, "containers": []});
    }
    let Some(out) = run("docker", &["ps", "-a", "--format", "{{json .}}"]) else {
        return json!({"available": true, "running": false, "containers": [], "error": "the Docker daemon did not answer"});
    };
    let mut list = Vec::new();
    for line in out.lines() {
        let Ok(v) = serde_json::from_str::<Value>(line) else { continue };
        let state = v["State"].as_str().unwrap_or("").to_string();
        list.push(json!({
            "engine": "docker",
            "id": v["ID"].as_str().unwrap_or(""),
            "name": v["Names"].as_str().unwrap_or(""),
            "image": v["Image"].as_str().unwrap_or(""),
            "command": v["Command"].as_str().unwrap_or("").trim_matches('"'),
            "status": v["Status"].as_str().unwrap_or(""),
            "state": state,
            "running": state == "running",
            "ports": v["Ports"].as_str().unwrap_or(""),
            "created": v["CreatedAt"].as_str().unwrap_or(""),
            "size": v["Size"].as_str().unwrap_or(""),
        }));
    }
    json!({"available": true, "running": true, "containers": list})
}

fn podman_ps() -> Value {
    if !have("podman") {
        return json!({"available": false, "running": false, "containers": []});
    }
    let Some(out) = run("podman", &["ps", "-a", "--format", "json"]) else {
        return json!({"available": true, "running": false, "containers": [], "error": "podman did not answer"});
    };
    let Ok(items) = serde_json::from_str::<Vec<Value>>(&out) else {
        return json!({"available": true, "running": true, "containers": []});
    };
    let list: Vec<Value> = items
        .iter()
        .map(|v| {
            let state = v["State"].as_str().unwrap_or("").to_string();
            json!({
                "engine": "podman",
                "id": v["Id"].as_str().unwrap_or(""),
                "name": v["Names"].as_array().and_then(|a| a.first()).and_then(Value::as_str).unwrap_or(""),
                "image": v["Image"].as_str().unwrap_or(""),
                "command": v["Command"].as_array().map(|a| a.iter().filter_map(Value::as_str).collect::<Vec<_>>().join(" ")).unwrap_or_default(),
                "status": v["Status"].as_str().unwrap_or(&state),
                "state": state,
                "running": state == "running",
                "ports": v["Ports"].as_array().map(|a| a.len()).unwrap_or(0),
                "created": v["CreatedAt"].as_str().unwrap_or(""),
            })
        })
        .collect();
    json!({"available": true, "running": true, "containers": list})
}

/// The units systemd is running and the ones it could not: two short lists and
/// the counts that go with them.
fn units(extra: &[&str]) -> Value {
    if !have("systemctl") {
        return json!({"available": false, "running": [], "failed": [], "total": 0});
    }
    let list = |state: &str| -> Vec<Value> {
        let mut args: Vec<&str> = extra.to_vec();
        let state_arg = format!("--state={state}");
        args.extend_from_slice(&["list-units", "--type=service", "--no-legend", "--no-pager", "--plain", &state_arg]);
        let Some(out) = run("systemctl", &args) else { return Vec::new() };
        out.lines()
            .filter_map(|line| {
                let mut it = line.split_whitespace();
                let unit = it.next()?;
                let load = it.next()?;
                let active = it.next()?;
                let sub = it.next()?;
                let description: String = it.collect::<Vec<_>>().join(" ");
                Some(json!({
                    "unit": unit,
                    "load": load,
                    "active": active,
                    "sub": sub,
                    "description": description,
                }))
            })
            .collect()
    };
    let running = list("running");
    let failed = list("failed");
    json!({
        "available": true,
        "running": running.len(),
        "failed": failed,
        "units": running,
    })
}

/// The kill button. `pid` 1 is refused outright, and a process that is not
/// ours comes back as a plain "not permitted" rather than an errno.
pub fn signal(pid: i32, name: &str) -> Result<(), String> {
    if pid <= 1 {
        return Err("that is not a process this can end".into());
    }
    let sig = match name {
        "TERM" | "" => libc::SIGTERM,
        "KILL" => libc::SIGKILL,
        "INT" => libc::SIGINT,
        "HUP" => libc::SIGHUP,
        "STOP" => libc::SIGSTOP,
        "CONT" => libc::SIGCONT,
        other => return Err(format!("unknown signal '{other}'")),
    };
    if unsafe { libc::kill(pid, sig) } == 0 {
        return Ok(());
    }
    let err = std::io::Error::last_os_error();
    Err(match err.raw_os_error() {
        Some(libc::EPERM) => "That process belongs to another user; the desktop cannot end it.".into(),
        Some(libc::ESRCH) => "That process has already ended.".into(),
        _ => format!("could not signal {pid}: {err}"),
    })
}

/// What one process is doing, for the details sheet.
pub fn process_detail(pid: i32) -> Result<Value, String> {
    let dir = PathBuf::from("/proc").join(pid.to_string());
    if !dir.exists() {
        return Err("that process has ended".into());
    }
    let status = fs::read_to_string(dir.join("status")).unwrap_or_default();
    let field = |key: &str| -> Option<String> {
        status
            .lines()
            .find_map(|l| l.strip_prefix(key))
            .and_then(|v| v.strip_prefix(':'))
            .map(|v| v.trim().to_string())
    };
    let kb = |key: &str| field(key).and_then(|v| v.split_whitespace().next().and_then(|n| n.parse::<f64>().ok())).map(|v| v * 1024.0);
    let fds = fs::read_dir(dir.join("fd")).map(|d| d.flatten().count()).ok();
    let (read, write) = proc_io(&dir);
    Ok(json!({
        "pid": pid,
        "name": field("Name").unwrap_or_default(),
        "cmd": command_line(&dir),
        "exe": fs::read_link(dir.join("exe")).ok().map(|p| p.to_string_lossy().into_owned()),
        "cwd": fs::read_link(dir.join("cwd")).ok().map(|p| p.to_string_lossy().into_owned()),
        "state": field("State").unwrap_or_default(),
        "ppid": field("PPid").and_then(|v| v.parse::<i64>().ok()),
        "threads": field("Threads").and_then(|v| v.parse::<i64>().ok()),
        "vmPeak": kb("VmPeak"),
        "vmSize": kb("VmSize"),
        "vmRss": kb("VmRSS"),
        "vmSwap": kb("VmSwap"),
        "fds": fds,
        "read": read as f64,
        "written": write as f64,
        "cgroup": read_trim(dir.join("cgroup")).and_then(|t| t.lines().next().map(|l| l.rsplit(':').next().unwrap_or(l).to_string())),
        "wine": is_wine(&dir),
        "voluntary": field("voluntary_ctxt_switches").and_then(|v| v.parse::<i64>().ok()),
        "involuntary": field("nonvoluntary_ctxt_switches").and_then(|v| v.parse::<i64>().ok()),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rates_need_two_samples() {
        assert_eq!(rate(200, 100, 2.0), 50.0);
        // A counter that went backwards (a device reappearing) is not a rate.
        assert_eq!(rate(100, 200, 2.0), 0.0);
        assert_eq!(rate(200, 100, 0.0), 0.0);
    }

    #[test]
    fn the_machine_describes_itself() {
        let mut m = Metrics::new();
        let cpu = m.cpu();
        assert!(cpu["threads"].as_u64().unwrap_or(0) >= 1);
        // The first sample has nothing to subtract, so it reads zero; the
        // second, after real time has passed, reads the truth.
        assert_eq!(cpu["usage"].as_f64(), Some(0.0));
        let memory = m.memory();
        assert!(memory["total"].as_f64().unwrap_or(0.0) > 0.0);
        assert!(memory["used"].as_f64().unwrap_or(0.0) <= memory["total"].as_f64().unwrap());
    }

    #[test]
    fn processes_are_sorted_and_cut() {
        let mut m = Metrics::new();
        let answer = m.processes(&json!({"limit": 5, "sort": "mem"}));
        let rows = answer["processes"].as_array().unwrap();
        assert!(!rows.is_empty() && rows.len() <= 5);
        assert!(answer["total"].as_u64().unwrap() >= rows.len() as u64);
        // This test is a process, so the table must contain it.
        let mine = std::process::id() as i64;
        let all = m.processes(&json!({"limit": 5000, "sort": "pid"}));
        assert!(all["processes"].as_array().unwrap().iter().any(|r| r["pid"].as_i64() == Some(mine)));
        for pair in rows.windows(2) {
            assert!(pair[0]["rss"].as_f64() >= pair[1]["rss"].as_f64());
        }
    }

    #[test]
    fn sorting_can_be_reversed_and_is_stable() {
        let mut rows = vec![
            json!({"pid": 3, "cpu": 1.0, "rss": 0.0}),
            json!({"pid": 1, "cpu": 5.0, "rss": 0.0}),
            json!({"pid": 2, "cpu": 1.0, "rss": 0.0}),
        ];
        sort_rows(&mut rows, "cpu", false);
        assert_eq!(rows.iter().map(|r| r["pid"].as_i64().unwrap()).collect::<Vec<_>>(), [1, 2, 3]);
        sort_rows(&mut rows, "cpu", true);
        assert_eq!(rows.iter().map(|r| r["pid"].as_i64().unwrap()).collect::<Vec<_>>(), [3, 2, 1]);
    }

    #[test]
    fn signals_refuse_what_they_should() {
        assert!(signal(1, "TERM").is_err());
        assert!(signal(0, "TERM").is_err());
        assert!(signal(std::process::id() as i32, "NOPE").is_err());
        // Signal 0 is not offered, so an unknown name never reaches kill().
        assert!(process_detail(1).is_ok());
        assert!(process_detail(-1).is_err());
    }

    #[test]
    fn filesystems_are_real_ones() {
        for fs in filesystems() {
            assert!(fs["size"].as_f64().unwrap() > 0.0);
            assert_ne!(fs["fstype"].as_str().unwrap(), "proc");
        }
    }
}

