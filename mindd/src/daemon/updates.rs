//! The update watcher: checks for package updates and Arch news on a timer,
//! rates the risk with rules and with the model, posts a notice, optionally
//! installs low-risk updates on its own, and records every run so the
//! health checks can verify the system afterwards.

use super::health;
use super::notices::{action, now};
use super::{llm, Daemon};
use crate::proto::{LastUpdate, NewsItem, Notice, PackageUpdate, UpdateStatus};
use anyhow::{anyhow, Result};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::time::Duration;

pub fn status_path(state_dir: &Path) -> PathBuf {
    state_dir.join("updates.json")
}

pub fn load(state_dir: &Path) -> UpdateStatus {
    let mut s: UpdateStatus = std::fs::read_to_string(status_path(state_dir)).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default();
    s.assessing = false;
    s.checking = false;
    s.applying = false;
    s
}

fn save(state_dir: &Path, s: &UpdateStatus) {
    let path = status_path(state_dir);
    let tmp = path.with_extension("json.tmp");
    if std::fs::write(&tmp, serde_json::to_string_pretty(s).unwrap_or_default()).is_ok() {
        let _ = std::fs::rename(&tmp, &path);
    }
}

async fn sh(cmd: &str, secs: u64) -> (bool, String) {
    let out = tokio::time::timeout(Duration::from_secs(secs), tokio::process::Command::new("/bin/sh").arg("-c").arg(cmd).env("LC_ALL", "C").env("NO_COLOR", "1").output()).await;
    match out {
        Ok(Ok(o)) => {
            let mut t = String::from_utf8_lossy(&o.stdout).into_owned();
            let e = String::from_utf8_lossy(&o.stderr);
            if !e.trim().is_empty() {
                t.push_str("\n[stderr]\n");
                t.push_str(e.trim_end());
            }
            (o.status.success(), t)
        }
        _ => (false, "timed out".into()),
    }
}

/// What a package's breakage would hit; drives the rule-based risk.
pub fn tag_for(name: &str) -> &'static str {
    let n = name;
    if n.starts_with("linux-mindos") || n == "linux-firmware" || n.starts_with("amd-ucode") || n.starts_with("intel-ucode") || n == "mkinitcpio" || n == "limine" || n == "dkms" {
        "kernel"
    } else if n.starts_with("nvidia") || n.starts_with("lib32-nvidia") || n.starts_with("mesa") || n.starts_with("lib32-mesa") || n.starts_with("vulkan-") || n.starts_with("lib32-vulkan") {
        "gpu"
    } else if n == "systemd" || n.starts_with("systemd-") || n == "glibc" || n == "lib32-glibc" || n == "pacman" || n == "dbus" || n == "greetd" || n == "seatd" || n == "btrfs-progs" || n == "snapper" || n == "openssl" {
        "core"
    } else if n.starts_with("mind") || n == "llama-cpp" || n.starts_with("ggml") || n.starts_with("mindos-") {
        "mindos"
    } else if n == "steam" || n.starts_with("wine") || n == "gamescope" || n == "gamemode" || n.starts_with("lib32-gamemode") || n.starts_with("mangohud") || n == "lutris" || n.starts_with("pipewire") || n.starts_with("lib32-pipewire") || n == "wireplumber" {
        "gaming"
    } else if n.starts_with("gtk4") || n.starts_with("webkitgtk") || n.starts_with("wayland") || n.starts_with("libinput") || n.starts_with("xorg-xwayland") {
        "graphics"
    } else {
        ""
    }
}

pub async fn check_packages() -> Result<Vec<PackageUpdate>> {
    let (_, out) = sh("checkupdates 2>&1; echo \"[rc $?]\"", 300).await;
    if out.contains("[rc 1]") && !out.contains(" -> ") {
        return Err(anyhow!("checkupdates failed: {}", out.lines().next().unwrap_or("").trim()));
    }
    let mut v = vec![];
    for line in out.lines() {
        // name 1.0-1 -> 1.1-1
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() == 4 && parts[2] == "->" {
            v.push(PackageUpdate { name: parts[0].into(), from: parts[1].into(), to: parts[3].into(), tag: tag_for(parts[0]).into() });
        }
    }
    Ok(v)
}

pub async fn fetch_news() -> Vec<NewsItem> {
    let (_, out) = sh("curl -sS --max-time 15 https://archlinux.org/feeds/news/ 2>/dev/null", 30).await;
    let mut items = vec![];
    // items are <item><title>..</title><link>..</link>..<pubDate>..</pubDate>
    for chunk in out.split("<item>").skip(1).take(12) {
        let get = |tag: &str| chunk.split(&format!("<{}>", tag)).nth(1).and_then(|s| s.split(&format!("</{}>", tag)).next()).map(|s| s.trim().to_string()).unwrap_or_default();
        let title = get("title").replace("&amp;", "&").replace("&quot;", "\"").replace("&#39;", "'");
        let date = get("pubDate");
        // "Tue, 02 Sep 2026 10:00:00 +0000" → 2026-09-02
        let date = chrono::DateTime::parse_from_rfc2822(&date).map(|d| d.format("%Y-%m-%d").to_string()).unwrap_or(date);
        items.push(NewsItem { title, date, url: get("link") });
    }
    items
}

fn news_since(news: &[NewsItem], since: u64) -> Vec<&NewsItem> {
    let since_day = chrono::DateTime::from_timestamp(since as i64, 0).map(|d| d.format("%Y-%m-%d").to_string()).unwrap_or_default();
    news.iter().filter(|n| n.date.as_str() >= since_day.as_str()).collect()
}

/// Risk from rules alone (the model refines it).
fn rule_assessment(s: &mut UpdateStatus, last_update_time: u64) {
    let mut warnings = vec![];
    let has = |t: &str| s.packages.iter().any(|p| p.tag == t);
    let names = |t: &str| s.packages.iter().filter(|p| p.tag == t).map(|p| p.name.clone()).collect::<Vec<_>>().join(", ");
    let mut risk = 0;
    if has("kernel") {
        risk = risk.max(2);
        warnings.push(format!("Kernel or boot stack changes ({}): a reboot is needed and the NVIDIA module is rebuilt for the new kernel.", names("kernel")));
        s.reboot = true;
    }
    if has("gpu") {
        risk = risk.max(2);
        warnings.push(format!("GPU driver or Vulkan changes ({}): games may behave differently; reboot afterwards.", names("gpu")));
        s.reboot = true;
    }
    if has("core") {
        risk = risk.max(2);
        warnings.push(format!("Core system packages ({}): pay attention to .pacnew files.", names("core")));
    }
    if has("mindos") {
        risk = risk.max(1);
        warnings.push(format!("MindOS components ({}): the desktop or the Mind may restart.", names("mindos")));
    }
    if has("gaming") {
        risk = risk.max(1);
        warnings.push(format!("Gaming stack ({}): Proton prefixes and Wine apps may need a first-run update.", names("gaming")));
    }
    let recent: Vec<String> = news_since(&s.news, last_update_time).iter().map(|n| format!("{} — {}", n.date, n.title)).collect();
    let manual = news_since(&s.news, last_update_time).iter().any(|n| {
        let t = n.title.to_lowercase();
        t.contains("manual intervention") || t.contains("requires manual") || t.contains("action required") || t.contains("breaking")
    });
    s.manual_intervention = manual;
    if manual {
        risk = 3;
        warnings.insert(0, "Arch news announces a manual intervention since your last update; read it before updating.".into());
    }
    if !recent.is_empty() {
        warnings.push(format!("Arch news since your last update: {}", recent.join("; ")));
    }
    if s.packages.len() > 150 {
        risk = risk.max(2);
        warnings.push(format!("A big batch ({} packages): more can go wrong at once; the pre-update snapshot is your way back.", s.packages.len()));
    }
    s.risk = ["low", "low", "medium", "high"][risk.min(3)].into();
    s.warnings = warnings;
    s.assessed_by_model = false;
    let n = s.packages.len();
    s.summary = if n == 0 {
        "The system is up to date.".into()
    } else {
        let tagged: Vec<String> = ["kernel", "gpu", "core", "mindos", "gaming", "graphics"].iter().filter(|t| has(t)).map(|t| t.to_string()).collect();
        format!("{} package{} to update{}.", n, if n == 1 { "" } else { "s" }, if tagged.is_empty() { " (applications and libraries only)".to_string() } else { format!(", touching {}", tagged.join(", ")) })
    };
}

/// Ask the model for a short assessment. Only the JSON it returns is used;
/// on any failure the rule assessment stands.
async fn model_assessment(d: &Daemon, s: &mut UpdateStatus) {
    if !d.is_ready() || d.is_sleeping() || game_running() || s.packages.is_empty() {
        return;
    }
    let pkgs: Vec<String> = s.packages.iter().take(120).map(|p| format!("{} {} -> {}{}", p.name, p.from, p.to, if p.tag.is_empty() { String::new() } else { format!(" [{}]", p.tag) })).collect();
    let news: Vec<String> = s.news.iter().take(8).map(|n| format!("{} {}", n.date, n.title)).collect();
    let prompt = format!(
        "You review a pending system update for a MindOS (Arch Linux, gaming) machine and tell the user, in plain words, how risky it is and what to watch out for. \
Rules already found: risk {rules_risk}, {warn_count} warning(s). Answer with one JSON object only, no prose: \
{{\"risk\": \"low\"|\"medium\"|\"high\", \"summary\": \"2-3 sentences for the user\", \"warnings\": [\"specific things to check or do, at most 4\"], \"reboot\": true|false}}.\n\n\
Pending packages ({n} total, tags in brackets show what a breakage would hit):\n{pkgs}\n\nArch news (newest first):\n{news}\n\nRule warnings:\n{warnings}",
        rules_risk = s.risk,
        warn_count = s.warnings.len(),
        n = s.packages.len(),
        pkgs = pkgs.join("\n"),
        news = if news.is_empty() { "(none)".to_string() } else { news.join("\n") },
        warnings = if s.warnings.is_empty() { "(none)".to_string() } else { s.warnings.join("\n") }
    );
    let messages = vec![llm::Message::system("You are Mind, the operator of a MindOS computer. You answer with JSON only."), llm::Message::user(prompt)];
    let res = tokio::time::timeout(Duration::from_secs(180), d.llm.chat(&messages, &[], |_| {}, || false)).await;
    let Ok(Ok((text, _))) = res else { return };
    let Some(v) = extract_json(&text) else { return };
    let risk = v["risk"].as_str().unwrap_or("").to_lowercase();
    if !matches!(risk.as_str(), "low" | "medium" | "high") {
        return;
    }
    // the model may lower the rule risk by one step at most, never below a manual intervention
    let order = |r: &str| match r { "low" => 0, "medium" => 1, _ => 2 };
    let rules = order(&s.risk);
    let mut chosen = order(&risk);
    if chosen + 1 < rules {
        chosen = rules - 1;
    }
    if s.manual_intervention {
        chosen = 2;
    }
    s.risk = ["low", "medium", "high"][chosen].into();
    if let Some(sum) = v["summary"].as_str().filter(|t| !t.trim().is_empty()) {
        s.summary = sum.trim().to_string();
    }
    let mut w: Vec<String> = v["warnings"].as_array().map(|a| a.iter().filter_map(|x| x.as_str()).map(|x| x.trim().to_string()).filter(|x| !x.is_empty()).take(4).collect()).unwrap_or_default();
    // keep the rule warnings the model dropped only when it gave none
    if w.is_empty() {
        w = s.warnings.clone();
    } else if s.manual_intervention {
        w.insert(0, "Arch news announces a manual intervention since your last update; read it before updating.".into());
    }
    s.warnings = w;
    if let Some(r) = v["reboot"].as_bool() {
        s.reboot = s.reboot || r;
    }
    s.assessed_by_model = true;
}

fn extract_json(text: &str) -> Option<Value> {
    let start = text.find('{')?;
    let end = text.rfind('}')?;
    serde_json::from_str(&text[start..=end]).ok()
}

fn post_notice(d: &Daemon, s: &UpdateStatus) {
    if s.packages.is_empty() {
        d.notices.dismiss("updates:available");
        return;
    }
    let level = match s.risk.as_str() { "high" => "danger", "medium" => "warn", _ => "info" };
    let n = s.packages.len();
    let title = match s.risk.as_str() {
        "high" => format!("{} updates — read the news first", n),
        "medium" => format!("{} updates — {} risk", n, s.risk),
        _ => format!("{} update{} ready", n, if n == 1 { "" } else { "s" }),
    };
    let mut body = s.summary.clone();
    if let Some(w) = s.warnings.first() {
        body.push('\n');
        body.push_str(w);
    }
    let mut actions = vec![action("Details", "settings", json!("updates"))];
    if !s.manual_intervention {
        actions.push(action("Install", "request", json!({"type": "apply_updates"})));
    }
    actions.push(action("Ask Mind", "chat", json!("Walk me through the pending updates: what they change, the risks, and whether I should install them now.")));
    d.notices.post(Notice { id: "updates:available".into(), level: level.into(), title, body, source: "updates".into(), time: 0, actions });
}

pub fn game_running() -> bool {
    std::fs::read_to_string("/run/mindos/perf/game").map(|s| s.trim() != "0" && !s.trim().is_empty()).unwrap_or(false)
}

/// What an assessment is about: the packages and their target versions.
fn package_key(packages: &[PackageUpdate]) -> Vec<String> {
    packages.iter().map(|p| format!("{}={}", p.name, p.to)).collect()
}

/// One full check: packages, news, rules, model. Posts the notice.
pub async fn check(d: &Daemon, assess: bool) -> UpdateStatus {
    {
        let mut s = d.updates.lock().unwrap();
        if s.checking {
            return s.clone();
        }
        s.checking = true;
        s.error.clear();
    }
    d.notify_updates();
    let last_time = health::load_last_update(&d.config.updates.last_update).map(|l| l.time).unwrap_or(0);
    let pkgs = check_packages().await;
    let news = fetch_news().await;
    let mut s = d.updates.lock().unwrap().clone();
    let previous = package_key(&s.packages);
    match pkgs {
        Ok(p) => s.packages = p,
        Err(e) => s.error = e.to_string(),
    }
    if !news.is_empty() {
        s.news = news;
    }
    s.checked_at = now();
    s.checking = false;
    let changed = package_key(&s.packages) != previous || s.risk.is_empty();
    if changed || !s.assessed_by_model {
        rule_assessment(&mut s, last_time);
    }
    s.last_update = health::load_last_update(&d.config.updates.last_update);
    s.auto_apply = d.auto_update();
    // The model's verdict costs up to three minutes of GPU: only for a list
    // it has not rated yet, and one at a time. A check that arrives while
    // one runs (Check now during the scheduled one) leaves the verdict to
    // land below, matched by package list.
    let assess_now = assess && d.config.updates.assess && (changed || !s.assessed_by_model) && !s.packages.is_empty() && !s.assessing;
    if assess_now {
        s.assessing = true;
    }
    *d.updates.lock().unwrap() = s.clone();
    if !assess_now {
        // the file is written once per check: here, or with the verdict
        save(&d.config.daemon.state_dir, &s);
    }
    d.notify_updates();
    if assess_now {
        model_assessment(d, &mut s).await;
        let mut cur = d.updates.lock().unwrap();
        // The verdict is about the list that was rated: keep it while that
        // is still the pending list (an apply or another check may have run
        // meanwhile); otherwise rate the new list soon.
        if package_key(&cur.packages) == package_key(&s.packages) && cur.manual_intervention == s.manual_intervention {
            cur.risk = s.risk.clone();
            cur.summary = s.summary.clone();
            cur.warnings = s.warnings.clone();
            cur.reboot = s.reboot;
            cur.assessed_by_model = s.assessed_by_model;
        } else if !cur.packages.is_empty() && !cur.assessed_by_model {
            d.check_now.notify_one();
        }
        cur.assessing = false;
        save(&d.config.daemon.state_dir, &cur);
        s = cur.clone();
        drop(cur);
        d.notify_updates();
    }
    post_notice(d, &s);
    d.audit.record("update_check", "", 0, json!({"packages": s.packages.len(), "risk": s.risk, "model": s.assessed_by_model, "error": s.error}));
    s
}

/// Install every pending update. snap-pac takes the snapshots; the pacman
/// hook records the transaction for the health verification.
pub async fn apply(d: &Daemon, uid: u32) -> Result<UpdateStatus> {
    {
        let mut s = d.updates.lock().unwrap();
        if s.applying {
            return Err(anyhow!("an update is already running"));
        }
        s.applying = true;
    }
    d.notify_updates();
    let started = now();
    let (ok, out) = sh("pacman -Syu --noconfirm 2>&1", 3600).await;
    let updated: Vec<String> = out.lines().filter_map(|l| l.strip_prefix("upgrading ").or_else(|| l.strip_prefix("installing ")).map(|s| s.trim_end_matches("...").trim().to_string())).collect();
    let n = updated.len();
    let pre = health::pre_snapshot_for(started).await;
    // the hook wrote the record; make sure the snapshot is on it
    let path = &d.config.updates.last_update;
    let mut lu = health::load_last_update(path).filter(|l| l.time >= started).unwrap_or(LastUpdate { time: now(), packages: updated.clone(), pre_snapshot: None, ok, verified: String::new(), report: String::new() });
    if lu.pre_snapshot.is_none() {
        lu.pre_snapshot = pre;
    }
    lu.ok = ok;
    health::save_last_update(path, &lu);
    d.audit.record("update_apply", "", uid, json!({"ok": ok, "packages": n, "snapshot": lu.pre_snapshot, "tail": out.lines().rev().take(5).collect::<Vec<_>>()}));
    {
        let mut s = d.updates.lock().unwrap();
        s.applying = false;
        s.last_update = Some(lu.clone());
        if ok {
            s.packages.clear();
            s.risk = "low".into();
            s.summary = "The system is up to date.".into();
            s.warnings.clear();
        }
        save(&d.config.daemon.state_dir, &s);
    }
    d.notify_updates();
    d.notices.dismiss("updates:available");
    if ok {
        let reboot = updated.iter().any(|p| p.starts_with("linux-mindos") || p.starts_with("nvidia"));
        let body = format!("{} package{} updated{}. The Mind checks the system in a minute and after the next boot.{}", n, if n == 1 { "" } else { "s" }, lu.pre_snapshot.map(|s| format!(" (snapshot {} taken before)", s)).unwrap_or_default(), if reboot { " A new kernel or GPU driver is installed: reboot when convenient." } else { "" });
        let mut actions = vec![action("Details", "settings", json!("updates"))];
        if reboot {
            actions.insert(0, action("Reboot", "request", json!({"type": "power", "action": "reboot"})));
        }
        d.notices.post(Notice { id: "updates:done".into(), level: "ok".into(), title: "Update installed".into(), body, source: "updates".into(), time: 0, actions });
    } else {
        let tail = out.lines().rev().take(6).collect::<Vec<_>>().into_iter().rev().collect::<Vec<_>>().join("\n");
        d.notices.post(Notice { id: "updates:failed".into(), level: "danger".into(), title: "The update failed".into(), body: tail.clone(), source: "updates".into(), time: 0, actions: vec![action("Ask Mind", "chat", json!(format!("The system update failed. pacman said:\n{}\nExplain what went wrong and fix it.", tail)))] });
    }
    // verify soon, not after the next timer tick
    let _ = tokio::time::timeout(Duration::from_secs(90), health::run_and_notify(d)).await;
    Ok(d.updates.lock().unwrap().clone())
}

/// The watcher loop.
pub async fn watch(d: std::sync::Arc<Daemon>) {
    let hours = d.config.updates.check_interval_hours;
    if hours == 0 {
        return;
    }
    // wait for the model (up to a few minutes) so the first check is assessed
    for _ in 0..60 {
        if d.is_ready() {
            break;
        }
        tokio::time::sleep(Duration::from_secs(5)).await;
    }
    tokio::time::sleep(Duration::from_secs(20)).await;
    loop {
        // Defer repository downloads and automatic inference while gaming.
        // Explicit checks still call check() directly from the request handler.
        while game_running() {
            tokio::time::sleep(Duration::from_secs(30)).await;
        }
        let s = check(&d, true).await;
        if d.auto_update() && !s.packages.is_empty() && s.risk == "low" && !s.manual_intervention && s.error.is_empty() && !game_running() {
            eprintln!("mindd: auto-update: {} low-risk packages", s.packages.len());
            let _ = apply(&d, 0).await;
        }
        let next = if d.updates.lock().unwrap().packages.is_empty() { hours * 3600 } else { (hours * 3600).min(6 * 3600) };
        tokio::select! {
            _ = tokio::time::sleep(Duration::from_secs(next)) => {}
            _ = d.check_now.notified() => {}
        }
    }
}
