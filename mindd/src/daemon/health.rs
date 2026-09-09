//! Health checks: deterministic findings the Mind turns into notices and
//! uses to verify the system after an update. No model involved; the model
//! explains a finding when the user asks.

use super::notices::{action, now};
use super::{sysinfo, Daemon};
use crate::proto::{Event, Finding, LastUpdate, Notice};
use serde_json::json;
use std::path::Path;
use std::time::Duration;

fn finding(id: &str, level: &str, title: &str, body: String) -> Finding {
    Finding { id: id.into(), level: level.into(), title: title.into(), body, actions: vec![] }
}

async fn sh(cmd: &str) -> String {
    let out = tokio::time::timeout(Duration::from_secs(60), tokio::process::Command::new("/bin/sh").arg("-c").arg(cmd).env("LC_ALL", "C").env("SYSTEMD_COLORS", "0").output()).await;
    match out {
        Ok(Ok(o)) => String::from_utf8_lossy(&o.stdout).trim().to_string(),
        _ => String::new(),
    }
}

/// Every check, in the order shown to the user.
pub async fn run() -> Vec<Finding> {
    let mut f = vec![];

    // failed units
    let failed = sh("systemctl --failed --no-legend --plain 2>/dev/null | awk '{print $1}'").await;
    let units: Vec<&str> = failed.lines().filter(|l| !l.is_empty()).collect();
    if !units.is_empty() {
        let mut x = finding("failed-units", "warn", &format!("{} service{} failed", units.len(), if units.len() == 1 { "" } else { "s" }), units.join(", "));
        x.actions.push(action("Explain", "chat", json!(format!("These systemd units failed: {}. Look at their logs, explain what is wrong and propose a fix.", units.join(", ")))));
        f.push(x);
    }

    // kernel: running vs installed
    let running = sysinfo::kernel();
    let installed = Path::new("/usr/lib/modules").join(&running).exists();
    if !running.is_empty() && !installed {
        let mut x = finding("kernel-stale", "warn", "Reboot to finish the kernel update", format!("The running kernel ({}) is no longer installed; drivers and modules cannot load until you reboot.", running));
        x.actions.push(action("Reboot", "request", json!({"type": "power", "action": "reboot"})));
        f.push(x);
    }

    // NVIDIA: GPU present, driver not loaded
    let gpus = sysinfo::gpus();
    let has_nvidia = gpus.iter().any(|g| g.to_lowercase().contains("nvidia"));
    if has_nvidia {
        let loaded = Path::new("/sys/module/nvidia").exists();
        let utils = sh("pacman -Q nvidia-utils 2>/dev/null | awk '{print $2}'").await;
        let module = sh("modinfo -F version nvidia 2>/dev/null | head -1").await;
        if !loaded {
            let mut x = finding("nvidia-not-loaded", "danger", "NVIDIA driver is not loaded", format!("An NVIDIA GPU is installed but the nvidia module is not loaded{}. Games will run on software rendering or not at all.", if module.is_empty() { " (no module built for this kernel)" } else { "" }));
            x.actions.push(action("Diagnose", "chat", json!("The NVIDIA GPU driver is not loaded. Check dkms status, the journal and modprobe, and fix it.")));
            f.push(x);
        } else if !utils.is_empty() && !module.is_empty() && !utils.starts_with(&module) && !module.starts_with(utils.split('-').next().unwrap_or("")) {
            let mut x = finding("nvidia-mismatch", "warn", "NVIDIA driver and libraries differ", format!("nvidia-utils {} is installed but the loaded kernel module is {}. Reboot after a driver update.", utils, module));
            x.actions.push(action("Reboot", "request", json!({"type": "power", "action": "reboot"})));
            f.push(x);
        }
    }

    // disk space
    // Immutable live media and Flatpak images are full by design.
    let df = sh("df -P -x tmpfs -x devtmpfs -x efivarfs -x overlay -x squashfs -x erofs -x iso9660 2>/dev/null | awk 'NR>1 {gsub(\"%\",\"\",$5); if ($5+0 >= 90) print $6\" \"$5\" \"$4}'").await;
    for line in df.lines() {
        let mut it = line.split_whitespace();
        let (Some(mnt), Some(pct), Some(avail_k)) = (it.next(), it.next(), it.next()) else { continue };
        let avail_g = avail_k.parse::<f64>().unwrap_or(0.0) / 1048576.0;
        let id = format!("disk-{}", mnt.trim_matches('/').replace('/', "-"));
        let id = if id == "disk-" { "disk-root".to_string() } else { id };
        let mut x = finding(&id, if avail_g < 2.0 { "danger" } else { "warn" }, &format!("{} is {}% full", mnt, pct), format!("{:.1} GB left. Updates and games need room; old snapshots and the package cache are the usual culprits.", avail_g));
        x.actions.push(action("Free space", "chat", json!(format!("{} is {}% full. Find what takes the space (package cache, old snapper snapshots, Steam shader caches, ~/.cache) and propose what to clean.", mnt, pct))));
        f.push(x);
    }

    // pacnew files
    let pacnew = sh("find /etc -name '*.pacnew' 2>/dev/null | head -20").await;
    let files: Vec<&str> = pacnew.lines().filter(|l| !l.is_empty()).collect();
    if !files.is_empty() {
        let mut x = finding("pacnew", "info", &format!("{} configuration file{} to merge", files.len(), if files.len() == 1 { "" } else { "s" }), files.join("\n"));
        x.actions.push(action("Show me", "chat", json!(format!("There are .pacnew files: {}. For each, show the difference from the current file and recommend what to do.", files.join(", ")))));
        f.push(x);
    }

    // kernel errors this boot (a rough signal; only the count and a sample)
    let errs = sh("journalctl -k -b -p err --no-pager -o cat 2>/dev/null | grep -v -e 'ACPI' -e 'BIOS' -e 'usb' -e 'i2c' | head -200").await;
    let lines: Vec<&str> = errs.lines().filter(|l| !l.trim().is_empty()).collect();
    if lines.len() >= 20 {
        let mut x = finding("kernel-errors", "info", &format!("{} kernel errors this boot", lines.len()), lines.iter().take(4).map(|l| l.chars().take(140).collect::<String>()).collect::<Vec<_>>().join("\n"));
        x.actions.push(action("Explain", "chat", json!("The kernel logged many errors this boot (journalctl -k -p err). Read them, group them, and say whether any matter for gaming or stability.")));
        f.push(x);
    }

    // mind
    if !Path::new("/run/mindos/mind.sock").exists() {
        f.push(finding("mindd-socket", "warn", "The Mind's socket is missing", "The desktop cannot reach mindd; check `systemctl status mindd`.".into()));
    }

    // snapshots: none at all means no way back (not asked on live media or without a config)
    if !Path::new("/run/archiso/bootmnt").exists() && Path::new("/etc/snapper/configs/root").exists() {
        let snaps = sh("snapper --no-dbus --csvout -c root list --columns number 2>/dev/null | grep -cE '^[1-9][0-9]*$' ").await;
        if snaps.trim() == "0" {
            f.push(finding("no-snapshots", "info", "No system snapshots yet", "The first update creates one; snapshots appear in the boot menu and `mindos-boot restore` goes back to one.".into()));
        }
    }
    f
}

pub fn load_last_update(path: &Path) -> Option<LastUpdate> {
    std::fs::read_to_string(path).ok().and_then(|s| serde_json::from_str(&s).ok())
}

pub fn save_last_update(path: &Path, lu: &LastUpdate) {
    if let Some(p) = path.parent() {
        let _ = std::fs::create_dir_all(p);
    }
    let tmp = path.with_extension("json.tmp");
    if std::fs::write(&tmp, serde_json::to_string_pretty(lu).unwrap_or_default()).is_ok() {
        let _ = std::fs::rename(&tmp, path);
    }
}

/// The pre-update snapper snapshot that best matches `time` (the "pre"
/// snapshot snap-pac took right before that pacman run).
pub async fn pre_snapshot_for(time: u64) -> Option<u64> {
    // CSV: the table output uses box-drawing separators and a locale date.
    let out = sh("snapper --no-dbus --csvout -c root list --columns number,type,date 2>/dev/null").await;
    let mut best: Option<(u64, i64)> = None;
    for line in out.lines() {
        let cols: Vec<&str> = line.split(',').map(str::trim).collect();
        if cols.len() < 3 || cols[1] != "pre" {
            continue;
        }
        let Ok(num) = cols[0].parse::<u64>() else { continue };
        let Ok(dt) = chrono::NaiveDateTime::parse_from_str(cols[2], "%Y-%m-%d %H:%M:%S") else { continue };
        let ts = dt.and_local_timezone(chrono::Local).single().map(|d| d.timestamp()).unwrap_or(0);
        let diff = (time as i64 - ts).abs();
        if diff < 3600 && best.map(|b| diff < b.1).unwrap_or(true) {
            best = Some((num, diff));
        }
    }
    best.map(|b| b.0)
}

/// Turn findings into notices (stable ids under "health:"), clearing the
/// ones that no longer apply. Also verifies the last update once.
pub async fn run_and_notify(d: &Daemon) -> Vec<Finding> {
    let findings = run().await;
    let mut keep = vec![];
    for x in &findings {
        if x.level == "ok" {
            continue;
        }
        let id = format!("health:{}", x.id);
        keep.push(id.clone());
        d.notices.post(Notice { id, level: x.level.clone(), title: x.title.clone(), body: x.body.clone(), source: "health".into(), time: 0, actions: x.actions.clone() });
    }
    d.notices.retain_prefix("health:", &keep);
    let checked_at = now();
    *d.last_health.lock().unwrap() = (checked_at, findings.clone());
    // Subscribed shells show the report on their Updates page.
    d.notices.broadcast(Event::Health { checked_at, findings: findings.clone() });
    verify_last_update(d, &findings).await;
    findings
}

/// After a pacman transaction (recorded by the hook or the Mind), say
/// whether the system still looks healthy and how to get back if not.
async fn verify_last_update(d: &Daemon, findings: &[Finding]) {
    let path = &d.config.updates.last_update;
    let Some(mut lu) = load_last_update(path) else { return };
    if !lu.verified.is_empty() {
        return;
    }
    // give a fresh boot a minute: services are still starting
    let uptime = std::fs::read_to_string("/proc/uptime").ok().and_then(|s| s.split_whitespace().next()?.parse::<f64>().ok()).unwrap_or(999.0);
    if uptime < 90.0 {
        return;
    }
    let touched_kernel = lu.packages.iter().any(|p| p.starts_with("linux-mindos") || p.starts_with("nvidia"));
    let running = sysinfo::kernel();
    let stale = !Path::new("/usr/lib/modules").join(&running).exists();
    if touched_kernel && stale {
        // cannot judge before the reboot; leave a gentle hint instead
        d.notices.post(Notice { id: "updates:reboot".into(), level: "info".into(), title: "Reboot to finish the update".into(), body: format!("The update on {} installed a new kernel or GPU driver. The Mind checks the system again after the reboot.", date(lu.time)), source: "updates".into(), time: 0, actions: vec![action("Reboot", "request", json!({"type": "power", "action": "reboot"}))] });
        return;
    }
    if lu.pre_snapshot.is_none() {
        lu.pre_snapshot = pre_snapshot_for(lu.time).await;
    }
    let problems: Vec<&Finding> = findings.iter().filter(|x| matches!(x.level.as_str(), "warn" | "danger") && x.id != "pacnew").collect();
    let n = lu.packages.len();
    if problems.is_empty() {
        lu.verified = "ok".into();
        lu.report = format!("{} package{} updated on {}; services, drivers and disks look fine.", n, if n == 1 { "" } else { "s" }, date(lu.time));
        d.notices.dismiss("updates:reboot");
        d.notices.post(Notice { id: "updates:verified".into(), level: "ok".into(), title: "Update checked: all good".into(), body: lu.report.clone(), source: "updates".into(), time: 0, actions: vec![action("Details", "settings", json!("updates"))] });
    } else {
        lu.verified = "problems".into();
        let list = problems.iter().map(|p| format!("• {}", p.title)).collect::<Vec<_>>().join("\n");
        // the question names the units/files, not just the headline, so the model has something to look at
        let detail = problems.iter().map(|p| match p.body.lines().next().filter(|l| !l.trim().is_empty()) { Some(l) => format!("{} ({})", p.title, l.trim()), None => p.title.clone() }).collect::<Vec<_>>().join("; ");
        let back = match lu.pre_snapshot {
            Some(s) => format!("\nSnapshot {} from before the update is in the boot menu; \"Roll back\" restores it.", s),
            None => String::new(),
        };
        lu.report = format!("After the update on {} ({} packages):\n{}{}", date(lu.time), n, list, back);
        let mut actions = vec![action("Explain", "chat", json!(format!("The system was updated on {} ({} packages: {}). Since then these problems appeared: {}. Which of the updated packages is the likely cause, and what should I do? Rolling back to snapshot {} is an option.", date(lu.time), n, lu.packages.iter().take(40).cloned().collect::<Vec<_>>().join(", "), detail, lu.pre_snapshot.map(|s| s.to_string()).unwrap_or_else(|| "(none)".into()))))];
        if let Some(s) = lu.pre_snapshot {
            actions.push(action("Roll back", "request", json!({"type": "rollback", "snapshot": s})));
        }
        actions.push(action("Details", "settings", json!("updates")));
        d.notices.dismiss("updates:reboot");
        d.notices.dismiss("updates:verified");
        d.notices.post(Notice { id: "updates:problems".into(), level: "warn".into(), title: "The last update may have broken something".into(), body: lu.report.clone(), source: "updates".into(), time: 0, actions });
    }
    save_last_update(path, &lu);
    d.audit.record("update_verified", "", 0, json!({"verified": lu.verified, "report": lu.report}));
}

pub fn date(t: u64) -> String {
    chrono::DateTime::from_timestamp(t as i64, 0).map(|d| d.with_timezone(&chrono::Local).format("%d %b %H:%M").to_string()).unwrap_or_default()
}
