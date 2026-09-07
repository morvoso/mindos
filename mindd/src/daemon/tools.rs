//! The tools the model can call. Each has a JSON schema, a category (for
//! autopilot rules) and a policy; run_command's policy is decided per call.

use super::policy;
use super::sysinfo;
use crate::config::Config;
use crate::proto::Policy;
use anyhow::{anyhow, Result};
use serde_json::{json, Value};
use std::time::Duration;
use tokio::process::Command;

pub struct ToolDef {
    pub name: &'static str,
    pub description: &'static str,
    pub parameters: Value,
    pub policy: Policy,
    pub category: &'static str,
}

const MAX_OUTPUT: usize = 12_000;

pub fn definitions() -> Vec<ToolDef> {
    let none = json!({"type": "object", "properties": {}});
    vec![
        ToolDef { name: "system_info", description: "Hardware, kernel, drivers, uptime and MindOS version of this machine.", parameters: none.clone(), policy: Policy::Observe, category: "info" },
        ToolDef { name: "gpu_info", description: "GPU model(s), driver in use, VRAM, Vulkan availability (lspci, nvidia-smi, vulkaninfo).", parameters: none.clone(), policy: Policy::Observe, category: "info" },
        ToolDef { name: "list_packages", description: "Installed packages (pacman -Q), optionally filtered by a substring.", parameters: json!({"type":"object","properties":{"filter":{"type":"string","description":"substring to match package names"}}}), policy: Policy::Observe, category: "packages" },
        ToolDef { name: "search_packages", description: "Search for packages in the MindOS/Arch repositories, on Flathub and in the AUR.", parameters: json!({"type":"object","properties":{"query":{"type":"string"}},"required":["query"]}), policy: Policy::Observe, category: "packages" },
        ToolDef { name: "package_info", description: "Where a package comes from (repository, Flathub, AUR), whether it is installed, version and description.", parameters: json!({"type":"object","properties":{"name":{"type":"string"}},"required":["name"]}), policy: Policy::Observe, category: "packages" },
        ToolDef { name: "check_updates", description: "List available package updates without installing anything.", parameters: none.clone(), policy: Policy::Observe, category: "update" },
        ToolDef { name: "apply_updates", description: "Update every installed package (pacman -Syu). Reports pacnew files and whether a reboot is needed.", parameters: none.clone(), policy: Policy::Change, category: "update" },
        ToolDef { name: "install_packages", description: "Install packages by plain name (e.g. discord, octopi). Tries the MindOS/Arch repositories, then Flathub, then the AUR (built locally, can take minutes) and reports which source was used.", parameters: json!({"type":"object","properties":{"packages":{"type":"array","items":{"type":"string"}}},"required":["packages"]}), policy: Policy::Change, category: "packages" },
        ToolDef { name: "remove_packages", description: "Remove installed packages or Flatpaks (and their unneeded dependencies).", parameters: json!({"type":"object","properties":{"packages":{"type":"array","items":{"type":"string"}}},"required":["packages"]}), policy: Policy::Change, category: "packages" },
        ToolDef { name: "service_status", description: "Status of a systemd unit, or a list of failed units when no unit is given.", parameters: json!({"type":"object","properties":{"unit":{"type":"string"}}}), policy: Policy::Observe, category: "services" },
        ToolDef { name: "service_control", description: "start, stop, restart, enable or disable a systemd unit.", parameters: json!({"type":"object","properties":{"unit":{"type":"string"},"action":{"type":"string","enum":["start","stop","restart","enable","disable","enable-now","disable-now"]}},"required":["unit","action"]}), policy: Policy::Change, category: "services" },
        ToolDef { name: "journal", description: "Recent log lines from the systemd journal, for the current boot. Filter by unit and/or priority (err, warning, info).", parameters: json!({"type":"object","properties":{"unit":{"type":"string"},"priority":{"type":"string"},"lines":{"type":"integer","default":80},"grep":{"type":"string"}}}), policy: Policy::Observe, category: "info" },
        ToolDef { name: "read_file", description: "Read a text file (configuration, logs, /proc, /sys). Up to 12000 characters.", parameters: json!({"type":"object","properties":{"path":{"type":"string"}},"required":["path"]}), policy: Policy::Observe, category: "files" },
        ToolDef { name: "write_file", description: "Write a configuration file (a backup of the previous version is kept as <path>.mindos-bak).", parameters: json!({"type":"object","properties":{"path":{"type":"string"},"content":{"type":"string"}},"required":["path","content"]}), policy: Policy::Change, category: "files" },
        ToolDef { name: "disk_usage", description: "Mounted filesystems, block devices and free space.", parameters: none.clone(), policy: Policy::Observe, category: "info" },
        ToolDef { name: "run_command", description: "Run a shell command as root and return its output. Read-only commands run immediately; anything that changes the system is shown to the user for confirmation first.", parameters: json!({"type":"object","properties":{"command":{"type":"string"},"timeout_secs":{"type":"integer","default":120}},"required":["command"]}), policy: Policy::Change, category: "shell" },
        ToolDef { name: "set_kernel_parameter", description: "Add or remove a kernel command-line parameter for future boots (GRUB and systemd-boot are updated).", parameters: json!({"type":"object","properties":{"add":{"type":"string","description":"parameter to add, e.g. nvidia_drm.modeset=1"},"remove":{"type":"string","description":"parameter (or prefix) to remove"}}}), policy: Policy::Change, category: "boot" },
        ToolDef { name: "reboot", description: "Reboot the machine (after confirming with the user).", parameters: none.clone(), policy: Policy::Change, category: "power" },
        ToolDef { name: "arch_news", description: "Latest Arch Linux news headlines: manual interventions required before updating are announced here.", parameters: none.clone(), policy: Policy::Observe, category: "update" },
        ToolDef { name: "update_status", description: "What the update watcher knows: pending packages with their risk tags, the risk assessment, warnings, Arch news, and the last update with its pre-update snapshot and verification result. Use `check: true` to run a fresh check first.", parameters: json!({"type":"object","properties":{"check":{"type":"boolean","default":false}}}), policy: Policy::Observe, category: "update" },
        ToolDef { name: "health_check", description: "Run the system health checks now: failed services, kernel/driver state, NVIDIA module, disk space, pacnew files, kernel errors. Returns the findings.", parameters: none.clone(), policy: Policy::Observe, category: "info" },
        ToolDef { name: "notices", description: "The notices the Mind has shown the user (update assessments, health findings, update reports). `dismiss` removes one by id, or all with \"*\".", parameters: json!({"type":"object","properties":{"dismiss":{"type":"string","description":"notice id to dismiss, or * for all"}}}), policy: Policy::Observe, category: "info" },
        ToolDef { name: "list_snapshots", description: "System snapshots (snapper) that the boot menu offers and `rollback` can restore: number, date, description.", parameters: none.clone(), policy: Policy::Observe, category: "boot" },
        ToolDef { name: "rollback", description: "Make snapshot N the system again (mindos-boot restore) and reboot. The current state is kept as a new snapshot so the rollback can be undone.", parameters: json!({"type":"object","properties":{"snapshot":{"type":"integer"}},"required":["snapshot"]}), policy: Policy::Change, category: "boot" },
        ToolDef { name: "performance_mode", description: "Read or switch the MindOS performance mode: balanced (default), performance (governor performance, sched_ext scx_lavd, no proactive compaction, NVIDIA persistence; GameMode switches here while a game runs) or quiet (powersave, no boost). Without `mode` it only reports the current state.", parameters: json!({"type":"object","properties":{"mode":{"type":"string","enum":["balanced","performance","quiet"]}}}), policy: Policy::Observe, category: "perf" },
        ToolDef { name: "dlss", description: "The DLSS/FSR/XeSS swapper (mindos-dlss), run as the user: `scan` lists the user's games and the upscaler DLLs they ship with versions; `library` the DLL versions on hand; `versions KIND` what can be downloaded; `download KIND VERSION|latest`; `swap GAME KIND VERSION|latest` (the original is kept as a backup); `restore GAME [KIND]`. Kinds: dlss, dlss_d, dlss_g, fsr_31_dx12, fsr_31_vk, xess, xess_fg, xess_dx11, xell.", parameters: json!({"type":"object","properties":{"args":{"type":"string","description":"the mindos-dlss command line, e.g. \"scan\" or \"swap Cyberpunk dlss latest\""}},"required":["args"]}), policy: Policy::Observe, category: "games" },
        ToolDef { name: "mind_sleep", description: "Unload the language model from the GPU (sleep) or load it again (wake). GameMode does this automatically while a game runs.", parameters: json!({"type":"object","properties":{"sleeping":{"type":"boolean"}},"required":["sleeping"]}), policy: Policy::Observe, category: "mind" },
    ]
}

pub fn openai_tools(defs: &[ToolDef], client_tools: &[crate::proto::ClientTool]) -> Vec<Value> {
    let mut v: Vec<Value> = defs
        .iter()
        .map(|d| json!({"type":"function","function":{"name":d.name,"description":d.description,"parameters":d.parameters}}))
        .collect();
    for t in client_tools {
        v.push(json!({"type":"function","function":{"name":t.name,"description":t.description,"parameters":t.parameters}}));
    }
    v
}

pub fn find<'a>(defs: &'a [ToolDef], name: &str) -> Option<&'a ToolDef> {
    defs.iter().find(|d| d.name == name)
}

/// Policy for a specific call (run_command depends on the command).
pub fn policy_for(def: &ToolDef, args: &Value, cfg: &Config) -> Policy {
    match def.name {
        "run_command" => policy::classify_command(args["command"].as_str().unwrap_or(""), &cfg.policy),
        "service_control" => {
            let unit = args["unit"].as_str().unwrap_or("");
            let action = args["action"].as_str().unwrap_or("");
            if policy::protected_unit(unit) && matches!(action, "stop" | "disable" | "disable-now") {
                Policy::Forbidden
            } else {
                Policy::Change
            }
        }
        "write_file" => {
            let p = args["path"].as_str().unwrap_or("");
            if p.starts_with("/etc/") || p.starts_with("/boot/") || p.starts_with("/home/") || p.starts_with("/var/lib/mindos/") || p.starts_with("/usr/local/") {
                Policy::Change
            } else {
                Policy::Forbidden
            }
        }
        "performance_mode" => {
            if args["mode"].as_str().map(|m| !m.is_empty()).unwrap_or(false) {
                Policy::Change
            } else {
                Policy::Observe
            }
        }
        "dlss" => {
            let a = args["args"].as_str().unwrap_or("").trim();
            let verb = a.split_whitespace().next().unwrap_or("");
            match verb {
                "scan" | "games" | "library" | "versions" | "kinds" => Policy::Observe,
                "download" | "swap" | "restore" | "import" | "delete" => Policy::Change,
                _ => Policy::Forbidden,
            }
        }
        "remove_packages" => {
            let pkgs = args["packages"].as_array().map(|a| a.iter().filter_map(|v| v.as_str()).collect::<Vec<_>>()).unwrap_or_default();
            if pkgs.iter().any(|p| ["base", "linux-mindos", "systemd", "pacman", "mindos-base", "mindd", "mindwm", "glibc"].contains(p)) {
                Policy::Forbidden
            } else {
                Policy::Change
            }
        }
        _ => def.policy,
    }
}

async fn sh(command: &str, timeout: Duration) -> Result<(bool, String)> {
    let mut cmd = Command::new("/bin/sh");
    cmd.arg("-c").arg(command).env("LC_ALL", "C").env("PAGER", "cat").env("SYSTEMD_PAGER", "").env("SYSTEMD_COLORS", "0").env("NO_COLOR", "1");
    cmd.stdin(std::process::Stdio::null()).kill_on_drop(true);
    let out = tokio::time::timeout(timeout, cmd.output()).await.map_err(|_| anyhow!("command timed out after {:?}: {}", timeout, command))??;
    let mut text = String::from_utf8_lossy(&out.stdout).into_owned();
    let err = String::from_utf8_lossy(&out.stderr);
    if !err.trim().is_empty() {
        if !text.is_empty() {
            text.push('\n');
        }
        text.push_str("[stderr]\n");
        text.push_str(err.trim_end());
    }
    Ok((out.status.success(), truncate(text)))
}

fn truncate(mut s: String) -> String {
    if s.len() > MAX_OUTPUT {
        // keep the head and the tail; the end of pacman output matters most
        let head: String = s.chars().take(MAX_OUTPUT / 3).collect();
        let tail: String = s.chars().rev().take(MAX_OUTPUT * 2 / 3).collect::<Vec<_>>().into_iter().rev().collect();
        s = format!("{}\n[... output truncated ...]\n{}", head, tail);
    }
    s
}

fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

fn pkg_list(args: &Value) -> Result<String> {
    let list = args["packages"].as_array().ok_or_else(|| anyhow!("packages must be an array"))?;
    let names: Vec<String> = list.iter().filter_map(|v| v.as_str()).map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect();
    if names.is_empty() {
        return Err(anyhow!("no packages given"));
    }
    for n in &names {
        if !n.chars().all(|c| c.is_ascii_alphanumeric() || "+-._@".contains(c)) {
            return Err(anyhow!("\"{}\" is not a package name (letters, digits, + - . _ @ only)", n));
        }
    }
    Ok(names.join(" "))
}

/// Who asked (for tools that act in the user's own files).
pub struct Caller {
    pub uid: u32,
}

fn user_name(uid: u32) -> Option<String> {
    let s = std::fs::read_to_string("/etc/passwd").ok()?;
    s.lines().find_map(|l| {
        let f: Vec<&str> = l.split(':').collect();
        (f.len() > 2 && f[2].parse::<u32>().ok() == Some(uid)).then(|| f[0].to_string())
    })
}

/// Run `command` as the calling user (their home, their Steam library).
async fn sh_as(caller: &Caller, command: &str, timeout: Duration) -> Result<(bool, String)> {
    if caller.uid == 0 || caller.uid == u32::MAX {
        return sh(command, timeout).await;
    }
    let user = user_name(caller.uid).ok_or_else(|| anyhow!("unknown uid {}", caller.uid))?;
    sh(&format!("runuser -u {} -- /bin/sh -c {}", shell_quote(&user), shell_quote(command)), timeout).await
}

/// Execute a tool. Returns a JSON value for the model.
pub async fn execute(name: &str, args: &Value, cfg: &Config, d: &super::Daemon, caller: &Caller) -> Result<Value> {
    let timeout = Duration::from_secs(cfg.daemon.tool_timeout_secs);
    let ok_out = |(ok, out): (bool, String)| json!({"ok": ok, "output": out});
    match name {
        "update_status" => {
            let s = if args["check"].as_bool().unwrap_or(false) { super::updates::check(d, true).await } else { d.updates.lock().unwrap().clone() };
            Ok(json!({"ok": true, "checked_at": s.checked_at, "risk": s.risk, "summary": s.summary, "warnings": s.warnings, "manual_intervention": s.manual_intervention, "reboot": s.reboot, "assessed_by_model": s.assessed_by_model, "auto_apply": s.auto_apply, "packages": s.packages.iter().map(|p| format!("{} {} -> {}{}", p.name, p.from, p.to, if p.tag.is_empty() { String::new() } else { format!(" [{}]", p.tag) })).collect::<Vec<_>>(), "news": s.news.iter().take(6).map(|n| format!("{} {}", n.date, n.title)).collect::<Vec<_>>(), "last_update": s.last_update, "error": s.error}))
        }
        "health_check" => {
            let findings = super::health::run_and_notify(d).await;
            if findings.is_empty() {
                Ok(json!({"ok": true, "output": "no problems found: services, kernel, GPU driver, disks and configuration look fine"}))
            } else {
                Ok(json!({"ok": true, "findings": findings.iter().map(|f| json!({"level": f.level, "title": f.title, "detail": f.body})).collect::<Vec<_>>()}))
            }
        }
        "notices" => {
            if let Some(id) = args["dismiss"].as_str().filter(|s| !s.trim().is_empty()) {
                let gone = d.notices.dismiss(id.trim());
                return Ok(json!({"ok": true, "dismissed": gone}));
            }
            Ok(json!({"ok": true, "notices": d.notices.list()}))
        }
        "list_snapshots" => Ok(ok_out(sh("mindos-boot list 2>&1 || snapper --no-dbus -c root list", timeout).await?)),
        "rollback" => {
            let n = args["snapshot"].as_u64().ok_or_else(|| anyhow!("snapshot number required"))?;
            let (ok, out) = sh(&format!("mindos-boot restore {} 2>&1", n), Duration::from_secs(600)).await?;
            if ok {
                tokio::spawn(async {
                    tokio::time::sleep(Duration::from_secs(5)).await;
                    let _ = Command::new("systemctl").arg("reboot").status().await;
                });
            }
            Ok(json!({"ok": ok, "output": format!("{}\n{}", out, if ok { "rebooting in 5 seconds" } else { "" })}))
        }
        "performance_mode" => {
            let mode = args["mode"].as_str().unwrap_or("").trim();
            if !mode.is_empty() {
                let (ok, out) = sh(&format!("mindos-perf set {}", shell_quote(mode)), Duration::from_secs(60)).await?;
                if !ok {
                    return Ok(json!({"ok": false, "output": out}));
                }
            }
            let (ok, out) = sh("mindos-perf status", Duration::from_secs(30)).await?;
            Ok(json!({"ok": ok, "output": out}))
        }
        "dlss" => {
            let a = args["args"].as_str().unwrap_or("").trim();
            if a.is_empty() || a.contains(|c: char| c == ';' || c == '&' || c == '|' || c == '`' || c == '$' || c == '>' || c == '<') {
                return Err(anyhow!("args must be a plain mindos-dlss command line"));
            }
            Ok(ok_out(sh_as(caller, &format!("mindos-dlss {}", a), Duration::from_secs(900)).await?))
        }
        "mind_sleep" => {
            let sleeping = args["sleeping"].as_bool().ok_or_else(|| anyhow!("sleeping required"))?;
            d.set_sleep(sleeping);
            Ok(json!({"ok": true, "output": if sleeping { "the model unloads now; the next question wakes it (takes a few seconds)" } else { "waking up" }}))
        }
        "system_info" => Ok(sysinfo::summary()),
        "gpu_info" => {
            let mut parts = vec![];
            parts.push(format!("lspci:\n{}", sysinfo::gpus().join("\n")));
            if let Some(s) = sysinfo::cmd("nvidia-smi", &["--query-gpu=name,driver_version,memory.total,memory.used,temperature.gpu,utilization.gpu", "--format=csv"]) {
                parts.push(format!("nvidia-smi:\n{}", s));
            } else {
                parts.push("nvidia-smi: not available".into());
            }
            let drm = sysinfo::cmd("sh", &["-c", "for d in /sys/class/drm/card*/device/driver; do [ -e \"$d\" ] && echo \"$(basename $(dirname $(dirname $d))): $(basename $(readlink $d))\"; done"]).unwrap_or_default();
            parts.push(format!("DRM drivers:\n{}", drm));
            if let Some(s) = sysinfo::cmd("sh", &["-c", "vulkaninfo --summary 2>/dev/null | grep -E 'deviceName|driverName|apiVersion' | head -12"]) {
                parts.push(format!("vulkan:\n{}", s));
            }
            Ok(json!({"ok": true, "output": parts.join("\n\n")}))
        }
        "list_packages" => {
            let f = args["filter"].as_str().unwrap_or("").trim();
            let cmd = if f.is_empty() {
                "echo \"$(pacman -Q | wc -l) packages installed ($(pacman -Qe | wc -l) explicitly)\"; echo; echo 'explicitly installed:'; pacman -Qe".to_string()
            } else {
                format!("echo \"$(pacman -Q | wc -l) packages installed\"; echo; m=$(pacman -Q | grep -i -- {q}); if [ -n \"$m\" ]; then echo \"matching {q}:\"; echo \"$m\"; else echo \"no installed package matches {q}\"; fi", q = shell_quote(f))
            };
            let (_, out) = sh(&cmd, timeout).await?;
            Ok(json!({"ok": true, "output": out}))
        }
        "search_packages" => {
            let q = args["query"].as_str().ok_or_else(|| anyhow!("query required"))?;
            Ok(ok_out(sh(&format!("mindos-pkg search {}", shell_quote(q)), timeout).await?))
        }
        "package_info" => {
            let n = args["name"].as_str().ok_or_else(|| anyhow!("name required"))?;
            Ok(ok_out(sh(&format!("mindos-pkg info {}", shell_quote(n.trim())), timeout).await?))
        }
        "check_updates" => Ok(ok_out(sh("checkupdates 2>&1; rc=$?; [ $rc -eq 2 ] && echo 'system is up to date'; true", timeout).await?)),
        "apply_updates" => {
            let (ok, out) = sh("pacman -Syu --noconfirm 2>&1; echo \"[exit $?]\"; echo; echo 'pacnew files:'; find /etc -name '*.pacnew' 2>/dev/null; echo; echo 'running kernel:'; uname -r; echo 'installed kernel:'; ls /usr/lib/modules/ 2>/dev/null", timeout).await?;
            Ok(json!({"ok": ok, "output": out}))
        }
        "install_packages" => {
            let list = pkg_list(args)?;
            Ok(ok_out(sh(&format!("mindos-pkg install {}", list), timeout).await?))
        }
        "remove_packages" => {
            let list = pkg_list(args)?;
            Ok(ok_out(sh(&format!("mindos-pkg remove {}", list), timeout).await?))
        }
        "service_status" => {
            let unit = args["unit"].as_str().unwrap_or("").trim();
            let cmd = if unit.is_empty() { "systemctl --failed --no-pager; echo; systemctl list-units --type=service --state=running --no-pager | head -40".to_string() } else { format!("systemctl status --no-pager -n 20 -- {}", shell_quote(unit)) };
            Ok(ok_out(sh(&cmd, timeout).await?))
        }
        "service_control" => {
            let unit = args["unit"].as_str().ok_or_else(|| anyhow!("unit required"))?;
            let action = args["action"].as_str().ok_or_else(|| anyhow!("action required"))?;
            let cmd = match action {
                "enable-now" => format!("systemctl enable --now -- {}", shell_quote(unit)),
                "disable-now" => format!("systemctl disable --now -- {}", shell_quote(unit)),
                a @ ("start" | "stop" | "restart" | "enable" | "disable") => format!("systemctl {} -- {}", a, shell_quote(unit)),
                _ => return Err(anyhow!("unknown action {}", action)),
            };
            Ok(ok_out(sh(&format!("{c} && systemctl status --no-pager -n 5 -- {u}", c = cmd, u = shell_quote(unit)), timeout).await?))
        }
        "journal" => {
            let lines = args["lines"].as_u64().unwrap_or(80).clamp(1, 500);
            let mut cmd = format!("journalctl -b --no-pager -o short -n {}", lines);
            if let Some(u) = args["unit"].as_str().filter(|s| !s.trim().is_empty()) {
                cmd.push_str(&format!(" -u {}", shell_quote(u)));
            }
            if let Some(p) = args["priority"].as_str().filter(|s| !s.trim().is_empty()) {
                cmd.push_str(&format!(" -p {}", shell_quote(p)));
            }
            if let Some(g) = args["grep"].as_str().filter(|s| !s.trim().is_empty()) {
                cmd.push_str(&format!(" -g {}", shell_quote(g)));
            }
            Ok(ok_out(sh(&cmd, timeout).await?))
        }
        "read_file" => {
            let p = args["path"].as_str().ok_or_else(|| anyhow!("path required"))?;
            if p.contains("shadow") || p.contains("gshadow") || p.starts_with("/proc/kcore") || p.contains("/.ssh/") || p.contains("private") {
                return Err(anyhow!("refusing to read {}", p));
            }
            let s = tokio::fs::read(p).await.map_err(|e| anyhow!("{}: {}", p, e))?;
            let text = String::from_utf8_lossy(&s).into_owned();
            Ok(json!({"ok": true, "path": p, "content": truncate(text)}))
        }
        "write_file" => {
            let p = args["path"].as_str().ok_or_else(|| anyhow!("path required"))?;
            let content = args["content"].as_str().ok_or_else(|| anyhow!("content required"))?;
            if std::path::Path::new(p).exists() {
                tokio::fs::copy(p, format!("{}.mindos-bak", p)).await?;
            }
            if let Some(dir) = std::path::Path::new(p).parent() {
                tokio::fs::create_dir_all(dir).await?;
            }
            tokio::fs::write(p, content).await.map_err(|e| anyhow!("{}: {}", p, e))?;
            Ok(json!({"ok": true, "path": p, "bytes": content.len()}))
        }
        "disk_usage" => Ok(ok_out(sh("df -h -x tmpfs -x devtmpfs -x efivarfs; echo; lsblk -o NAME,SIZE,TYPE,FSTYPE,MOUNTPOINTS", timeout).await?)),
        "run_command" => {
            let c = args["command"].as_str().ok_or_else(|| anyhow!("command required"))?;
            let t = Duration::from_secs(args["timeout_secs"].as_u64().unwrap_or(120).clamp(1, cfg.daemon.tool_timeout_secs));
            let (ok, out) = sh(c, t).await?;
            Ok(json!({"ok": ok, "output": out}))
        }
        "set_kernel_parameter" => {
            let add = args["add"].as_str().unwrap_or("").trim().to_string();
            let remove = args["remove"].as_str().unwrap_or("").trim().to_string();
            if add.is_empty() && remove.is_empty() {
                return Err(anyhow!("add or remove required"));
            }
            let out = set_kernel_parameter(&add, &remove).await?;
            Ok(json!({"ok": true, "output": out}))
        }
        "reboot" => {
            tokio::spawn(async {
                tokio::time::sleep(Duration::from_secs(3)).await;
                let _ = Command::new("systemctl").arg("reboot").status().await;
            });
            Ok(json!({"ok": true, "output": "rebooting in 3 seconds"}))
        }
        "arch_news" => {
            let (ok, out) = sh("curl -sS --max-time 10 https://archlinux.org/feeds/news/ | grep -oE '<title>[^<]+</title>|<pubDate>[^<]+</pubDate>' | sed -e 's/<[^>]*>//g' | head -20", timeout).await?;
            Ok(json!({"ok": ok, "output": out}))
        }
        _ => Err(anyhow!("unknown tool {}", name)),
    }
}

/// Edit the kernel command line in every place MindOS supports.
async fn set_kernel_parameter(add: &str, remove: &str) -> Result<String> {
    let mut report = vec![];
    // /etc/kernel/cmdline (systemd-boot / UKI / mkinitcpio) and GRUB default
    for path in ["/etc/kernel/cmdline"] {
        if let Ok(s) = tokio::fs::read_to_string(path).await {
            let new = edit_cmdline(s.trim(), add, remove);
            tokio::fs::write(path, format!("{}\n", new)).await?;
            report.push(format!("{}: {}", path, new));
        }
    }
    let grub = "/etc/default/grub";
    if let Ok(s) = tokio::fs::read_to_string(grub).await {
        let mut out = String::new();
        let mut changed = false;
        for line in s.lines() {
            if let Some(rest) = line.strip_prefix("GRUB_CMDLINE_LINUX_DEFAULT=") {
                let cur = rest.trim().trim_matches('"');
                let new = edit_cmdline(cur, add, remove);
                out.push_str(&format!("GRUB_CMDLINE_LINUX_DEFAULT=\"{}\"\n", new));
                report.push(format!("{}: {}", grub, new));
                changed = true;
            } else {
                out.push_str(line);
                out.push('\n');
            }
        }
        if changed {
            tokio::fs::write(grub, out).await?;
            if std::path::Path::new("/boot/grub/grub.cfg").exists() {
                let (ok, o) = sh("grub-mkconfig -o /boot/grub/grub.cfg 2>&1 | tail -3", Duration::from_secs(120)).await?;
                report.push(format!("grub-mkconfig: {} {}", if ok { "ok" } else { "failed" }, o));
            }
        }
    }
    // systemd-boot entries
    if let Ok(mut rd) = tokio::fs::read_dir("/boot/loader/entries").await {
        while let Ok(Some(e)) = rd.next_entry().await {
            let p = e.path();
            if let Ok(s) = tokio::fs::read_to_string(&p).await {
                let mut out = String::new();
                for line in s.lines() {
                    if let Some(rest) = line.strip_prefix("options") {
                        out.push_str(&format!("options {}\n", edit_cmdline(rest.trim(), add, remove)));
                    } else {
                        out.push_str(line);
                        out.push('\n');
                    }
                }
                tokio::fs::write(&p, out).await?;
                report.push(format!("{}: updated", p.display()));
            }
        }
    }
    if report.is_empty() {
        return Err(anyhow!("no kernel command line configuration found"));
    }
    Ok(report.join("\n"))
}

fn edit_cmdline(cur: &str, add: &str, remove: &str) -> String {
    let key = |p: &str| p.split('=').next().unwrap_or("").to_string();
    let mut params: Vec<String> = cur.split_whitespace().map(|s| s.to_string()).collect();
    if !remove.is_empty() {
        params.retain(|p| p != remove && key(p) != key(remove));
    }
    if !add.is_empty() {
        params.retain(|p| key(p) != key(add));
        params.push(add.to_string());
    }
    params.join(" ")
}
