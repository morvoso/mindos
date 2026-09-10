//! Facts about the machine, used in the system prompt and by tools.

use serde_json::{json, Value};
use std::process::Command;
use std::sync::OnceLock;

pub fn read(p: &str) -> String {
    std::fs::read_to_string(p).unwrap_or_default()
}

pub fn cmd(prog: &str, args: &[&str]) -> Option<String> {
    let out = Command::new(prog).args(args).output().ok()?;
    if !out.status.success() && out.stdout.is_empty() {
        return None;
    }
    Some(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

pub fn os_release() -> (String, String) {
    let s = read("/etc/os-release");
    let get = |k: &str| s.lines().find_map(|l| l.strip_prefix(&format!("{}=", k))).map(|v| v.trim_matches('"').to_string()).unwrap_or_default();
    (get("PRETTY_NAME"), get("VERSION_ID"))
}

pub fn cpu_model() -> String {
    read("/proc/cpuinfo").lines().find_map(|l| l.strip_prefix("model name")).map(|l| l.trim_start_matches([' ', ':', '\t']).to_string()).unwrap_or_default()
}

pub fn mem_total_gib() -> f64 {
    read("/proc/meminfo").lines().find_map(|l| l.strip_prefix("MemTotal:")).and_then(|v| v.trim().split_whitespace().next()?.parse::<f64>().ok()).map(|kib| kib / 1024.0 / 1024.0).unwrap_or(0.0)
}

/// The running kernel (what `uname -r` prints). Fixed until the next boot,
/// which restarts the daemon.
pub fn kernel() -> String {
    static KERNEL: OnceLock<String> = OnceLock::new();
    KERNEL
        .get_or_init(|| {
            let k = read("/proc/sys/kernel/osrelease");
            if k.trim().is_empty() { cmd("uname", &["-r"]).unwrap_or_default() } else { k.trim().to_string() }
        })
        .clone()
}

/// The graphics adapters (lspci), probed once: the cards do not change
/// while the daemon runs, and the health checks ask every half hour.
pub fn gpus() -> Vec<String> {
    static GPUS: OnceLock<Vec<String>> = OnceLock::new();
    GPUS.get_or_init(|| cmd("lspci", &["-d", "::0300"]).into_iter().chain(cmd("lspci", &["-d", "::0302"]).into_iter()).flat_map(|s| s.lines().map(|l| l.to_string()).collect::<Vec<_>>()).collect()).clone()
}

/// The configured timezone ("America/New_York"), read from the symlink
/// systemd keeps at /etc/localtime. It is the only hint the machine has
/// about where its user is, which is what a question about the weather or
/// about what is open right now actually depends on.
pub fn timezone() -> String {
    std::fs::read_link("/etc/localtime")
        .ok()
        .and_then(|p| {
            let p = p.to_string_lossy().into_owned();
            p.split_once("zoneinfo/").map(|(_, tz)| tz.to_string())
        })
        .unwrap_or_default()
}

pub fn nvidia_driver() -> Option<String> {
    let v = read("/sys/module/nvidia/version");
    if v.trim().is_empty() {
        None
    } else {
        Some(v.trim().to_string())
    }
}

pub fn summary() -> Value {
    let (pretty, ver) = os_release();
    json!({
        "os": pretty,
        "os_version": ver,
        "kernel": kernel(),
        "hostname": read("/etc/hostname").trim(),
        "cpu": cpu_model(),
        "cpus": std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1),
        "memory_gib": (mem_total_gib() * 10.0).round() / 10.0,
        "gpus": gpus(),
        "nvidia_driver": nvidia_driver(),
        "timezone": timezone(),
        "uptime": cmd("uptime", &["-p"]).unwrap_or_default(),
        "cmdline": read("/proc/cmdline").trim(),
        "scheduler": read("/sys/kernel/sched_ext/root/ops").trim(),
    })
}

pub fn summary_text() -> String {
    let s = summary();
    let mut out = String::new();
    for (k, v) in s.as_object().unwrap() {
        let v = match v {
            Value::String(s) => s.clone(),
            Value::Null => continue,
            v => v.to_string(),
        };
        if v.is_empty() {
            continue;
        }
        out.push_str(&format!("- {}: {}\n", k, v));
    }
    out
}
