//! What may run, and what must be confirmed first.

use crate::config::PolicyConfig;
use crate::proto::Policy;
use serde_json::Value;

/// Programs a run_command may execute without confirmation (read-only).
const OBSERVE_COMMANDS: &[&str] = &[
    "ls", "cat", "head", "tail", "grep", "rg", "find", "wc", "du", "df", "free", "uptime", "uname", "id", "whoami",
    "hostname", "hostnamectl", "date", "lspci", "lsusb", "lsblk", "lscpu", "lsmod", "lsof", "dmesg", "journalctl",
    "systemctl", "loginctl", "nvidia-smi", "vulkaninfo", "glxinfo", "eglinfo", "wayland-info", "pacman", "checkupdates",
    "pactree", "paccache", "ip", "ss", "ping", "dig", "nmcli", "iw", "sensors", "nproc", "ps", "top", "pgrep", "pidof",
    "stat", "file", "which", "type", "env", "printenv", "readlink", "realpath", "sha256sum", "md5sum", "git", "cargo",
    "rustc", "python", "python3", "pip", "gcc", "clang", "make", "ldd", "objdump", "strings", "steam", "gamescope",
    "mangohud", "gamemoded", "gamemodelist", "protontricks", "wine", "winetricks", "flatpak", "getent", "true",
    "echo", "printf", "test", "sort", "uniq", "cut", "tr", "awk", "sed", "jq", "xargs", "diff", "cmp", "tree",
    "mount", "findmnt", "zramctl", "swapon", "sysctl", "modinfo", "udevadm", "bootctl", "efibootmgr", "fwupdmgr",
    "mkinitcpio", "dkms", "smartctl", "nvme", "hdparm", "fstrim", "timedatectl", "localectl", "resolvectl", "curl", "wget",
];

/// Substrings that are never allowed anywhere in a command.
const FORBIDDEN: &[&str] = &[
    "rm -rf /", "rm -rf /*", "mkfs", "dd if=", "> /dev/sd", "> /dev/nvme", ":(){", "shred", "wipefs", "sgdisk --zap",
    "parted", "fdisk", "cryptsetup luksFormat", "chmod -R 777 /", "chown -R", "/dev/mem", "mind.jsonl", "passwd root",
    "userdel", "rm -rf /boot", "rm -rf /usr", "rm -rf /etc", "rm -rf /var", "rm -rf /home", "pacman -Rns base",
    "pacman -R base", "systemctl mask mindd", "systemctl disable mindd", "kill -9 -1",
];

/// Systemd services that are part of MindOS and must not be stopped by the model.
const PROTECTED_UNITS: &[&str] = &["mindd", "greetd", "seatd", "systemd-", "dbus"];

/// Shell operators that turn a read-only command into something else.
fn has_mutating_shell(cmd: &str) -> bool {
    // redirections and command substitution can write; pipes are fine
    cmd.contains('>') || cmd.contains("$(") || cmd.contains('`') || cmd.contains("<(")
}

pub fn classify_command(cmd: &str, cfg: &PolicyConfig) -> Policy {
    let lower = cmd.to_lowercase();
    if FORBIDDEN.iter().any(|f| lower.contains(&f.to_lowercase())) {
        return Policy::Forbidden;
    }
    if has_mutating_shell(cmd) {
        return Policy::Change;
    }
    // every pipeline stage / && segment must be an observe command
    for stage in cmd.split(|c| c == '|' || c == ';' || c == '&').map(str::trim).filter(|s| !s.is_empty()) {
        let stage = stage.trim_start_matches("sudo ").trim();
        let prog = stage.split_whitespace().next().unwrap_or("");
        let prog = prog.rsplit('/').next().unwrap_or(prog);
        let allowed = OBSERVE_COMMANDS.contains(&prog) || cfg.extra_observe_commands.iter().any(|c| c == prog);
        if !allowed {
            return Policy::Change;
        }
        // read-only programs with mutating sub-commands
        let rest = stage.split_whitespace().skip(1).collect::<Vec<_>>().join(" ");
        let mutating = match prog {
            "pacman" => rest.contains("-S") && !rest.contains("-Ss") && !rest.contains("-Si") && !rest.contains("-Sl") && !rest.contains("-Sg") || rest.contains("-R") || rest.contains("-U"),
            "systemctl" => ["start", "stop", "restart", "enable", "disable", "mask", "unmask", "reboot", "poweroff", "kill", "daemon-reload", "set-default", "isolate"].iter().any(|k| rest.split_whitespace().any(|w| w == *k)),
            "git" => ["push", "reset", "clean", "checkout", "rebase", "merge", "commit", "rm", "stash"].iter().any(|k| rest.split_whitespace().any(|w| w == *k)),
            "cargo" => rest.starts_with("install") || rest.starts_with("publish"),
            "pip" => rest.starts_with("install") || rest.starts_with("uninstall"),
            "flatpak" => !rest.starts_with("list") && !rest.starts_with("info") && !rest.starts_with("search"),
            "nmcli" => rest.contains(" up") || rest.contains(" down") || rest.contains("add") || rest.contains("modify") || rest.contains("delete") || rest.contains("connect"),
            "ip" => ["add", "del", "set", "flush", "change", "replace"].iter().any(|k| rest.split_whitespace().any(|w| w == *k)),
            "sysctl" => rest.contains("-w") || rest.contains('='),
            "mount" => !rest.is_empty(),
            "swapon" => !rest.contains("--show") && !rest.contains("-s"),
            "fstrim" | "hdparm" | "mkinitcpio" | "dkms" | "efibootmgr" | "fwupdmgr" | "bootctl" => !(rest.contains("-l") || rest.contains("status") || rest.contains("list") || rest.contains("get-") || rest.is_empty() && prog == "bootctl"),
            "sed" => rest.contains("-i"),
            "curl" | "wget" => rest.contains("-o") || rest.contains("-O") || rest.contains("--output"),
            "steam" | "gamescope" | "wine" | "winetricks" | "protontricks" => true,
            "timedatectl" | "localectl" | "hostnamectl" => rest.starts_with("set-"),
            "udevadm" => rest.starts_with("trigger") || rest.starts_with("control"),
            _ => false,
        };
        if mutating {
            return Policy::Change;
        }
    }
    Policy::Observe
}

pub fn protected_unit(unit: &str) -> bool {
    PROTECTED_UNITS.iter().any(|p| unit.starts_with(p))
}

/// Should this call run without asking? `category` is the tool's category.
pub fn needs_confirmation(policy: Policy, category: &str, autopilot: bool, cfg: &PolicyConfig) -> bool {
    match policy {
        Policy::Observe => false,
        Policy::Forbidden => true,
        Policy::Change => !(autopilot || cfg.autopilot || cfg.autopilot_categories.iter().any(|c| c == category)),
    }
}

pub fn args_summary(args: &Value) -> String {
    match args {
        Value::Object(m) => m.iter().map(|(k, v)| format!("{}={}", k, short(v))).collect::<Vec<_>>().join(" "),
        v => short(v),
    }
}

fn short(v: &Value) -> String {
    let s = match v {
        Value::String(s) => s.clone(),
        v => v.to_string(),
    };
    if s.chars().count() > 120 {
        format!("{}…", s.chars().take(117).collect::<String>())
    } else {
        s
    }
}
