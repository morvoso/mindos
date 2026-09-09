//! System helpers: power, statistics, audio (WirePlumber), network, battery.

use std::os::unix::process::CommandExt;
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

/// Whether `cmd` is on the PATH. The answer is kept for ten minutes: the
/// samplers ask many times a minute between them, and a helper installed
/// or removed meanwhile shows up on the next check.
fn have(cmd: &str) -> bool {
    use std::collections::HashMap;
    use std::sync::{Mutex, OnceLock};
    static KNOWN: OnceLock<Mutex<HashMap<String, (Instant, bool)>>> = OnceLock::new();
    let known = KNOWN.get_or_init(Default::default);
    if let Some((at, found)) = known.lock().ok().and_then(|m| m.get(cmd).copied()) {
        if at.elapsed() < Duration::from_secs(600) {
            return found;
        }
    }
    let found = std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).any(|d| d.join(cmd).is_file()))
        .unwrap_or(false);
    if let Ok(mut m) = known.lock() {
        m.insert(cmd.to_string(), (Instant::now(), found));
    }
    found
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
    /// The last snapshot, handed out again for half a second so every
    /// widget that asks in the same moment shows the same numbers.
    snapshot: Option<(Instant, Value)>,
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
        if let Some((at, v)) = &self.snapshot {
            if at.elapsed() < Duration::from_millis(500) {
                return v.clone();
            }
        }
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
        let v = json!({
            "cpu": (cpu * 10.0).round() / 10.0,
            "cores": cores,
            "memUsed": mem_total - mem_avail,
            "memTotal": mem_total,
            "gpu": self.gpu.as_ref().map(|(_, v)| v.clone()),
            "load": load,
            "uptime": uptime,
        });
        self.snapshot = Some((Instant::now(), v.clone()));
        v
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

/// PipeWire's own change feed, `pactl subscribe` (part of pipewire-pulse):
/// one line per event. `None` when it is not available.
pub fn audio_events() -> Option<std::process::Child> {
    if !have("pactl") {
        return None;
    }
    let mut cmd = Command::new("pactl");
    cmd.arg("subscribe")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null());
    unsafe {
        // The feed ends with the shell, whichever way the shell ends.
        cmd.pre_exec(|| {
            libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGTERM);
            Ok(())
        });
    }
    cmd.spawn().ok()
}

/// Whether a `pactl subscribe` line ("Event 'change' on sink #48") concerns
/// a sink or the server (the default sink changing): the only events that
/// can move the volume the bar shows. Streams, clients and sources are not.
pub fn audio_event_matters(line: &str) -> bool {
    line.rsplit(" on ")
        .next()
        .map(|what| what.starts_with("sink #") || what.starts_with("server"))
        .unwrap_or(false)
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

// ---------------------------------------------------------------- WireGuard
//
// Tunnels are NetworkManager connections of type `wireguard`, driven through
// nmcli: the shell never touches keys or the interface itself, and NetworkManager
// keeps the tunnel up across shell restarts. `nmcli -t` separates fields with
// ':' and escapes a ':' inside a value as '\:'.

fn nmcli_fields(line: &str) -> Vec<String> {
    let mut fields = vec![String::new()];
    let mut chars = line.chars();
    while let Some(c) = chars.next() {
        match c {
            '\\' => {
                if let Some(next) = chars.next() {
                    fields.last_mut().unwrap().push(next);
                }
            }
            ':' => fields.push(String::new()),
            _ => fields.last_mut().unwrap().push(c),
        }
    }
    fields
}

/// One property of a connection, from `nmcli -t connection show <id>`.
fn nmcli_prop<'a>(lines: &'a str, key: &str) -> Option<&'a str> {
    lines
        .lines()
        .find_map(|l| l.strip_prefix(key).and_then(|rest| rest.strip_prefix(':')))
        .map(str::trim)
}

fn nmcli_error(out: &std::process::Output, what: &str) -> String {
    let err = String::from_utf8_lossy(&out.stderr);
    let msg = err.trim().lines().last().unwrap_or("").trim_start_matches("Error: ").to_string();
    if msg.is_empty() {
        format!("{what} failed")
    } else {
        format!("{what}: {msg}")
    }
}

fn nmcli_run(args: &[&str], what: &str) -> Result<String, String> {
    let out = Command::new("nmcli").args(args).output().map_err(|e| format!("nmcli: {e}"))?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    } else {
        Err(nmcli_error(&out, what))
    }
}

/// True when the argument is a NetworkManager connection UUID (the only way
/// the UI names a tunnel to the host, so a name can never be mistaken for an
/// nmcli option).
fn is_uuid(s: &str) -> bool {
    s.len() == 36 && s.bytes().enumerate().all(|(i, b)| match i {
        8 | 13 | 18 | 23 => b == b'-',
        _ => b.is_ascii_hexdigit(),
    })
}

/// Every WireGuard tunnel NetworkManager knows, active ones first.
pub fn vpn_list() -> Value {
    if !have("nmcli") {
        return json!({"available": false, "tunnels": []});
    }
    let Some(out) = run("nmcli", &["-t", "-f", "NAME,UUID,TYPE,DEVICE,ACTIVE,AUTOCONNECT", "connection", "show"]) else {
        return json!({"available": false, "tunnels": []});
    };
    let mut tunnels = Vec::new();
    for line in out.lines() {
        let f = nmcli_fields(line);
        if f.len() < 6 || f[2] != "wireguard" {
            continue;
        }
        let (name, uuid, device, active, autoconnect) = (&f[0], &f[1], &f[3], f[4] == "yes", f[5] == "yes");
        let detail = run("nmcli", &["-t", "-f", "connection.interface-name,ipv4.addresses,wireguard.peers,GENERAL.STATE", "connection", "show", uuid])
            .unwrap_or_default();
        // A deleted profile whose tunnel is still coming down is listed for a
        // moment longer, without any settings; nothing can be done with it.
        if !detail.lines().any(|l| l.starts_with("connection.interface-name:")) {
            continue;
        }
        let iface = nmcli_prop(&detail, "connection.interface-name")
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .or_else(|| Some(device.clone()).filter(|s| !s.is_empty()));
        let address = nmcli_prop(&detail, "ipv4.addresses")
            .and_then(|s| s.split(',').next())
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());
        // `wireguard.peers` lists every peer as `<key> allowed-ips=… endpoint=host:port …`.
        let peers = nmcli_prop(&detail, "wireguard.peers").unwrap_or("");
        let endpoint = peers
            .split_whitespace()
            .find_map(|w| w.strip_prefix("endpoint="))
            .map(str::to_string);
        let peer_count = peers.split(',').filter(|p| !p.trim().is_empty()).count();
        let state = nmcli_prop(&detail, "GENERAL.STATE").unwrap_or("").to_string();
        tunnels.push(json!({
            "id": uuid, "name": name, "iface": iface, "address": address,
            "endpoint": endpoint, "peers": peer_count,
            "active": active, "activating": state == "activating", "autoconnect": autoconnect,
        }));
    }
    tunnels.sort_by(|a, b| {
        let act = |v: &Value| v["active"].as_bool().unwrap_or(false);
        act(b).cmp(&act(a)).then_with(|| a["name"].as_str().unwrap_or("").to_lowercase().cmp(&b["name"].as_str().unwrap_or("").to_lowercase()))
    });
    json!({"available": true, "tunnels": tunnels})
}

/// Bring a tunnel up or down.
pub fn vpn_set_active(id: &str, up: bool) -> Result<(), String> {
    if !is_uuid(id) {
        return Err("not a tunnel id".into());
    }
    let verb = if up { "up" } else { "down" };
    nmcli_run(&["connection", verb, "uuid", id], if up { "Could not connect" } else { "Could not disconnect" }).map(|_| ())
}

/// Whether NetworkManager brings the tunnel up on its own at start-up.
pub fn vpn_set_autoconnect(id: &str, on: bool) -> Result<(), String> {
    if !is_uuid(id) {
        return Err("not a tunnel id".into());
    }
    nmcli_run(
        &["connection", "modify", "uuid", id, "connection.autoconnect", if on { "yes" } else { "no" }],
        "Could not change the tunnel",
    )
    .map(|_| ())
}

/// Forget a tunnel (its keys go with it).
pub fn vpn_remove(id: &str) -> Result<(), String> {
    if !is_uuid(id) {
        return Err("not a tunnel id".into());
    }
    nmcli_run(&["connection", "delete", "uuid", id], "Could not remove the tunnel").map(|_| ())
}

/// Import a `wg-quick` style configuration file. The connection takes the
/// file's name (`office.conf` → `office`) and NetworkManager brings it up at
/// once; returns the new connection's UUID.
pub fn vpn_import(path: &std::path::Path) -> Result<String, String> {
    if !path.is_file() {
        return Err("That file does not exist".into());
    }
    let text = std::fs::read_to_string(path).map_err(|e| format!("Could not read the file: {e}"))?;
    if !text.lines().any(|l| l.trim().eq_ignore_ascii_case("[interface]")) {
        return Err("That is not a WireGuard configuration (no [Interface] section)".into());
    }
    let out = nmcli_run(&["connection", "import", "type", "wireguard", "file", &path.to_string_lossy()], "Could not import the tunnel")?;
    // "Connection 'office' (uuid) successfully added."
    let uuid = out
        .split('(')
        .nth(1)
        .and_then(|s| s.split(')').next())
        .map(str::to_string)
        .filter(|s| is_uuid(s))
        .unwrap_or_default();
    Ok(uuid)
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

/// The user's home directory, from $HOME.
pub fn home_dir() -> Option<std::path::PathBuf> {
    std::env::var_os("HOME").map(std::path::PathBuf::from).filter(|p| p.is_absolute())
}

pub fn host_name() -> String {
    static NAME: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    NAME.get_or_init(|| {
        std::fs::read_to_string("/etc/hostname")
            .map(|s| s.trim().to_string())
            .ok()
            .filter(|s| !s.is_empty())
            .or_else(|| std::env::var("HOSTNAME").ok())
            .unwrap_or_else(|| "mindos".into())
    })
    .clone()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nmcli_terse_fields_unescape() {
        assert_eq!(nmcli_fields("office:33b8:wireguard:office:yes:yes"), ["office", "33b8", "wireguard", "office", "yes", "yes"]);
        assert_eq!(nmcli_fields(r"Home\: VPN:1:wireguard::no:no"), ["Home: VPN", "1", "wireguard", "", "no", "no"]);
        assert_eq!(nmcli_fields(""), [""]);
    }

    #[test]
    fn audio_feed_lines() {
        assert!(audio_event_matters("Event 'change' on sink #48"));
        assert!(audio_event_matters("Event 'change' on server"));
        assert!(!audio_event_matters("Event 'new' on sink-input #12"));
        assert!(!audio_event_matters("Event 'change' on client #3"));
        assert!(!audio_event_matters(""));
    }

    #[test]
    fn nmcli_props_and_ids() {
        let detail = "connection.interface-name:wg0\nipv4.addresses:10.66.0.2/24\nwireguard.peers:KEY= allowed-ips=0.0.0.0/0 endpoint=vpn.example.net:51820\n";
        assert_eq!(nmcli_prop(detail, "connection.interface-name"), Some("wg0"));
        assert_eq!(nmcli_prop(detail, "wireguard.peers").unwrap().split_whitespace().find_map(|w| w.strip_prefix("endpoint=")), Some("vpn.example.net:51820"));
        assert_eq!(nmcli_prop(detail, "ipv6.addresses"), None);
        assert!(is_uuid("33b8e36a-5b64-42fb-8f29-230894f4d8b4"));
        assert!(!is_uuid("office"));
        assert!(!is_uuid("--ask"));
        assert!(vpn_set_active("--ask", true).is_err());
    }
}
