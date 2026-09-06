//! System helpers: power, statistics, audio (WirePlumber), network, battery.

use std::process::Command;
use std::time::{Duration, Instant};

use serde_json::{json, Value};

fn run(cmd: &str, args: &[&str]) -> Option<String> {
    let out = Command::new(cmd).args(args).output().ok()?;
    if !out.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&out.stdout).into_owned())
}

fn have(cmd: &str) -> bool {
    std::env::var_os("PATH")
        .map(|p| {
            std::env::split_paths(&p).any(|d| d.join(cmd).is_file())
        })
        .unwrap_or(false)
}

pub fn power(action: &str) -> Result<(), String> {
    let args: &[&str] = match action {
        "shutdown" | "poweroff" => &["poweroff"],
        "reboot" | "restart" => &["reboot"],
        "suspend" | "sleep" => &["suspend"],
        "hibernate" => &["hibernate"],
        other => return Err(format!("unknown power action '{other}'")),
    };
    tracing::info!(action, "power action");
    let status = Command::new("systemctl")
        .args(args)
        .stdin(std::process::Stdio::null())
        .status()
        .map_err(|e| format!("systemctl: {e}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("systemctl {} failed ({status})", args[0]))
    }
}

/// End the login session when the compositor cannot do it for us.
pub fn logout_fallback() -> Result<(), String> {
    if let Ok(id) = std::env::var("XDG_SESSION_ID") {
        let status = Command::new("loginctl")
            .args(["terminate-session", &id])
            .status()
            .map_err(|e| format!("loginctl: {e}"))?;
        if status.success() {
            return Ok(());
        }
    }
    Err("cannot log out: no compositor connection and no XDG_SESSION_ID".into())
}

#[derive(Default)]
pub struct Stats {
    last_cpu: Option<(u64, u64)>,
    gpu: Option<(Instant, Value)>,
}

impl Stats {
    fn cpu_percent(&mut self) -> f64 {
        let Ok(text) = std::fs::read_to_string("/proc/stat") else { return 0.0 };
        let Some(line) = text.lines().next() else { return 0.0 };
        let nums: Vec<u64> = line
            .split_whitespace()
            .skip(1)
            .filter_map(|n| n.parse().ok())
            .collect();
        if nums.len() < 4 {
            return 0.0;
        }
        let idle = nums[3] + nums.get(4).copied().unwrap_or(0);
        let total: u64 = nums.iter().sum();
        let pct = match self.last_cpu {
            Some((lt, li)) if total > lt => {
                let dt = (total - lt) as f64;
                let di = (idle - li) as f64;
                ((dt - di) / dt * 100.0).clamp(0.0, 100.0)
            }
            _ => 0.0,
        };
        self.last_cpu = Some((total, idle));
        pct
    }

    /// Sample the GPU (nvidia-smi for now); cached for two seconds.
    pub fn gpu_sample(&mut self) -> Option<Value> {
        if let Some((at, v)) = &self.gpu {
            if at.elapsed() < Duration::from_secs(2) {
                return Some(v.clone());
            }
        }
        if !have("nvidia-smi") {
            return None;
        }
        let out = run(
            "nvidia-smi",
            &[
                "--query-gpu=utilization.gpu,temperature.gpu,memory.used,memory.total,name",
                "--format=csv,noheader,nounits",
            ],
        )?;
        let line = out.lines().next()?;
        let parts: Vec<&str> = line.split(',').map(|s| s.trim()).collect();
        if parts.len() < 5 {
            return None;
        }
        let v = json!({
            "util": parts[0].parse::<f64>().unwrap_or(0.0),
            "temp": parts[1].parse::<f64>().unwrap_or(0.0),
            "mem": parts[2].parse::<f64>().unwrap_or(0.0),
            "memTotal": parts[3].parse::<f64>().unwrap_or(0.0),
            "name": parts[4],
        });
        self.gpu = Some((Instant::now(), v.clone()));
        Some(v)
    }

    pub fn snapshot(&mut self) -> Value {
        let cpu = self.cpu_percent();
        let mut mem_total = 0.0;
        let mut mem_avail = 0.0;
        if let Ok(text) = std::fs::read_to_string("/proc/meminfo") {
            for line in text.lines() {
                let mut it = line.split_whitespace();
                match (it.next(), it.next()) {
                    (Some("MemTotal:"), Some(v)) => mem_total = v.parse::<f64>().unwrap_or(0.0) * 1024.0,
                    (Some("MemAvailable:"), Some(v)) => mem_avail = v.parse::<f64>().unwrap_or(0.0) * 1024.0,
                    _ => {}
                }
            }
        }
        let load: Vec<f64> = std::fs::read_to_string("/proc/loadavg")
            .ok()
            .map(|t| t.split_whitespace().take(3).filter_map(|v| v.parse::<f64>().ok()).collect())
            .unwrap_or_default();
        let uptime = std::fs::read_to_string("/proc/uptime")
            .ok()
            .and_then(|t| t.split_whitespace().next().and_then(|v| v.parse::<f64>().ok()))
            .unwrap_or(0.0);
        let cores = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1);
        json!({
            "cpu": (cpu * 10.0).round() / 10.0,
            "cores": cores,
            "memUsed": mem_total - mem_avail,
            "memTotal": mem_total,
            "gpu": self.gpu.as_ref().map(|(_, v)| v.clone()),
            "load": load,
            "uptime": uptime,
        })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Audio {
    pub volume: f64,
    pub muted: bool,
}

pub fn audio_get() -> Option<Audio> {
    let out = run("wpctl", &["get-volume", "@DEFAULT_AUDIO_SINK@"])?;
    // "Volume: 0.45 [MUTED]"
    let volume = out
        .split_whitespace()
        .nth(1)
        .and_then(|v| v.parse::<f64>().ok())?;
    Some(Audio {
        volume,
        muted: out.contains("MUTED"),
    })
}

pub fn audio_value(a: &Option<Audio>) -> Value {
    match a {
        Some(a) => json!({"volume": a.volume, "muted": a.muted, "sink": "default", "available": true}),
        None => json!({"volume": 0.0, "muted": false, "sink": null, "available": false}),
    }
}

pub fn audio_set(volume: f64) -> Result<(), String> {
    let v = volume.clamp(0.0, 1.5);
    let status = Command::new("wpctl")
        .args(["set-volume", "@DEFAULT_AUDIO_SINK@", &format!("{v:.2}")])
        .status()
        .map_err(|e| format!("wpctl: {e}"))?;
    if status.success() {
        Ok(())
    } else {
        Err("wpctl set-volume failed".into())
    }
}

pub fn audio_toggle_mute() -> Result<(), String> {
    let status = Command::new("wpctl")
        .args(["set-mute", "@DEFAULT_AUDIO_SINK@", "toggle"])
        .status()
        .map_err(|e| format!("wpctl: {e}"))?;
    if status.success() {
        Ok(())
    } else {
        Err("wpctl set-mute failed".into())
    }
}

pub fn network_status() -> Value {
    if have("nmcli") {
        if let Some(out) = run("nmcli", &["-t", "-f", "TYPE,STATE,CONNECTION,DEVICE", "device"]) {
            let mut best: Option<(u8, Value)> = None;
            for line in out.lines() {
                let parts: Vec<&str> = line.split(':').collect();
                if parts.len() < 4 {
                    continue;
                }
                let (kind, state, conn, dev) = (parts[0], parts[1], parts[2], parts[3]);
                if !state.starts_with("connected") {
                    continue;
                }
                let (rank, kind_name) = match kind {
                    "wifi" => (1, "wifi"),
                    "ethernet" => (0, "ethernet"),
                    "loopback" | "bridge" | "tun" | "wireguard" => continue,
                    _ => (2, "other"),
                };
                let ip = ip_for(dev);
                let v = json!({
                    "connected": true, "kind": kind_name, "ssid": if kind == "wifi" { Some(conn) } else { None },
                    "iface": dev, "ip": ip, "connection": conn,
                });
                if best.as_ref().map(|(r, _)| rank < *r).unwrap_or(true) {
                    best = Some((rank, v));
                }
            }
            if let Some((_, v)) = best {
                return v;
            }
            return json!({"connected": false, "kind": "none", "iface": null});
        }
    }
    // sysfs fallback
    if let Ok(entries) = std::fs::read_dir("/sys/class/net") {
        for e in entries.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            if name == "lo" {
                continue;
            }
            let state = std::fs::read_to_string(e.path().join("operstate")).unwrap_or_default();
            if state.trim() == "up" {
                let wifi = e.path().join("wireless").exists();
                return json!({
                    "connected": true, "kind": if wifi { "wifi" } else { "ethernet" },
                    "iface": name, "ip": ip_for(&name),
                });
            }
        }
    }
    json!({"connected": false, "kind": "none", "iface": null})
}

fn ip_for(dev: &str) -> Option<String> {
    let out = run("ip", &["-4", "-o", "addr", "show", "dev", dev])?;
    out.split_whitespace()
        .skip_while(|w| *w != "inet")
        .nth(1)
        .map(|s| s.split('/').next().unwrap_or(s).to_string())
}

pub fn battery_status() -> Value {
    let Ok(entries) = std::fs::read_dir("/sys/class/power_supply") else {
        return json!({"present": false});
    };
    for e in entries.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        let p = e.path();
        let kind = std::fs::read_to_string(p.join("type")).unwrap_or_default();
        if kind.trim() != "Battery" && !name.starts_with("BAT") {
            continue;
        }
        let capacity = std::fs::read_to_string(p.join("capacity"))
            .ok()
            .and_then(|t| t.trim().parse::<i32>().ok());
        let Some(percent) = capacity else { continue };
        let status = std::fs::read_to_string(p.join("status")).unwrap_or_default();
        let status = status.trim().to_string();
        let charging = status == "Charging";
        return json!({
            "present": true, "percent": percent, "charging": charging,
            "status": status, "full": status == "Full",
        });
    }
    json!({"present": false})
}

pub fn user_name() -> String {
    std::env::var("USER").or_else(|_| std::env::var("LOGNAME")).unwrap_or_else(|_| "user".into())
}

pub fn host_name() -> String {
    std::fs::read_to_string("/etc/hostname")
        .map(|s| s.trim().to_string())
        .ok()
        .filter(|s| !s.is_empty())
        .or_else(|| std::env::var("HOSTNAME").ok())
        .unwrap_or_else(|| "mindos".into())
}
