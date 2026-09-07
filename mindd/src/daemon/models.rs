//! Model management: which GGUF is the default, the download catalog
//! (`/etc/mindos/model-catalog.json`), runtime preferences (thinking on/off)
//! and downloads into `models_dir` (through `curl`, which is always present).
//!
//! The choice of model is a symlink, `models_dir/default.gguf`, which
//! `select_model` honours first; that keeps hand-made setups working and lets
//! a user drop in any GGUF they like. Changing the choice or the thinking
//! preference tells the supervisor (through `Daemon::restart`) to restart
//! llama-server.

use crate::config::ModelConfig;
use crate::proto::{CatalogEntry, DownloadState, Event, ModelEntry};
use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

use super::Daemon;

/// Runtime preferences the Settings app can change (`state_dir/mind-prefs.json`).
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Prefs {
    /// Let the model think (reason) before answering. Off by default: the
    /// bundled Qwen3.5 spends its whole token budget thinking about trivial
    /// questions when this is on, and most answers do not need it.
    pub thinking: Option<bool>,
    /// Install low-risk updates automatically (Settings › Updates).
    pub auto_update: Option<bool>,
}

pub fn prefs_path(state_dir: &Path) -> PathBuf {
    state_dir.join("mind-prefs.json")
}

pub fn load_prefs(state_dir: &Path) -> Prefs {
    std::fs::read_to_string(prefs_path(state_dir))
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

pub fn save_prefs(state_dir: &Path, prefs: &Prefs) -> Result<()> {
    std::fs::create_dir_all(state_dir).ok();
    let path = prefs_path(state_dir);
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_string_pretty(prefs)?)?;
    std::fs::rename(&tmp, &path)?;
    Ok(())
}

pub fn default_link(cfg: &ModelConfig) -> PathBuf {
    cfg.models_dir.join("default.gguf")
}

/// Where `default.gguf` points, if it exists and is not dangling.
pub fn default_target(cfg: &ModelConfig) -> Option<PathBuf> {
    let link = default_link(cfg);
    let target = std::fs::read_link(&link).ok()?;
    let abs = if target.is_absolute() { target } else { cfg.models_dir.join(target) };
    abs.is_file().then_some(abs)
}

/// Read the catalog file; an unreadable or missing catalog is just empty.
pub fn load_catalog(path: &Path) -> Vec<CatalogEntry> {
    let Ok(text) = std::fs::read_to_string(path) else { return vec![] };
    match serde_json::from_str::<Vec<CatalogEntry>>(&text) {
        Ok(list) => list,
        Err(e) => {
            eprintln!("mindd: ignoring {}: {}", path.display(), e);
            vec![]
        }
    }
}

/// The GGUF files in `models_dir` (excluding the `default.gguf` link itself).
pub fn installed(cfg: &ModelConfig) -> Vec<ModelEntry> {
    let mut list = Vec::new();
    let Ok(rd) = std::fs::read_dir(&cfg.models_dir) else { return list };
    for e in rd.flatten() {
        let p = e.path();
        let name = e.file_name().to_string_lossy().into_owned();
        if name == "default.gguf" || !name.ends_with(".gguf") {
            continue;
        }
        let Ok(m) = std::fs::metadata(&p) else { continue }; // follows links; skips dangling ones
        if !m.is_file() {
            continue;
        }
        list.push(ModelEntry { file: name, path: p.to_string_lossy().into_owned(), size: m.len(), active: false });
    }
    list.sort_by(|a, b| a.file.to_lowercase().cmp(&b.file.to_lowercase()));
    list
}

fn valid_file_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() < 200
        && name.ends_with(".gguf")
        && name != "default.gguf"
        && !name.starts_with('.')
        && name.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_' | '+'))
}

/// Hosts a download may come from when the URL is not in the catalog.
fn allowed_url(url: &str, catalog: &[CatalogEntry]) -> bool {
    if catalog.iter().any(|c| c.url == url) {
        return true;
    }
    let Some(rest) = url.strip_prefix("https://") else { return false };
    let host = rest.split('/').next().unwrap_or("");
    host == "huggingface.co" || host == "hf.co" || host.ends_with(".huggingface.co") || host.ends_with(".hf.co")
}

impl Daemon {
    /// The current model choice and everything the Settings app shows.
    pub fn models_event(&self) -> Event {
        let cfg = &self.config.model;
        let running = self.model_path.lock().unwrap().clone();
        let chosen = default_target(cfg);
        let current = running.clone().or_else(|| chosen.clone());
        let mut models = installed(cfg);
        for m in &mut models {
            m.active = current.as_ref().map(|c| same_file(c, Path::new(&m.path))).unwrap_or(false);
        }
        // A model outside models_dir (a user's own file) still shows up.
        if let Some(c) = &current {
            if !models.iter().any(|m| m.active) {
                if let Ok(meta) = std::fs::metadata(c) {
                    models.push(ModelEntry {
                        file: c.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default(),
                        path: c.to_string_lossy().into_owned(),
                        size: meta.len(),
                        active: true,
                    });
                }
            }
        }
        let mut catalog = load_catalog(&cfg.catalog);
        for c in &mut catalog {
            c.installed = models.iter().any(|m| m.file == c.file);
        }
        Event::Models {
            current: current.map(|p| p.to_string_lossy().into_owned()),
            model: self.model_name(),
            ready: self.is_ready(),
            auto: chosen.is_none() && cfg.path == "auto",
            thinking: self.thinking.load(Ordering::Relaxed),
            models_dir: cfg.models_dir.to_string_lossy().into_owned(),
            external: cfg.external_url.is_some(),
            gpu_memory: super::llm::gpu_memory_bytes(),
            models,
            catalog,
            download: self.download.lock().unwrap().clone(),
        }
    }

    /// Point `default.gguf` at `path` ("" or "auto" removes the link so the
    /// automatic choice applies again) and restart the model server.
    pub fn set_model(&self, path: &str) -> Result<()> {
        let cfg = &self.config.model;
        if cfg.external_url.is_some() {
            return Err(anyhow!("an external model server is configured; set the model there"));
        }
        let link = default_link(cfg);
        if path.is_empty() || path == "auto" {
            let _ = std::fs::remove_file(&link);
        } else {
            let p = if path.contains('/') { PathBuf::from(path) } else { cfg.models_dir.join(path) };
            let meta = std::fs::metadata(&p).with_context(|| format!("{} is not readable", p.display()))?;
            if !meta.is_file() || p.extension().map(|e| e != "gguf").unwrap_or(true) {
                return Err(anyhow!("{} is not a .gguf model file", p.display()));
            }
            let p = std::fs::canonicalize(&p).unwrap_or(p);
            std::fs::create_dir_all(&cfg.models_dir).ok();
            let tmp = cfg.models_dir.join(".default.gguf.tmp");
            let _ = std::fs::remove_file(&tmp);
            std::os::unix::fs::symlink(&p, &tmp).context("creating the default.gguf link")?;
            std::fs::rename(&tmp, &link).context("replacing default.gguf")?;
        }
        self.request_restart();
        Ok(())
    }

    pub fn set_thinking(&self, enabled: bool) -> Result<()> {
        let mut prefs = load_prefs(&self.config.daemon.state_dir);
        prefs.thinking = Some(enabled);
        save_prefs(&self.config.daemon.state_dir, &prefs)?;
        if self.thinking.swap(enabled, Ordering::Relaxed) != enabled {
            self.request_restart();
        }
        Ok(())
    }

    fn request_restart(&self) {
        self.ready.store(false, Ordering::Release);
        *self.model_name.lock().unwrap() = "(loading)".into();
        self.restart.notify_one();
    }

    /// Start downloading `url` into `models_dir/file`. Progress is stored in
    /// `self.download` and also streamed to `events` until the download ends.
    pub fn start_download(self: &Arc<Self>, url: String, file: String, size: u64, use_after: bool, events: tokio::sync::mpsc::UnboundedSender<Event>) -> Result<()> {
        let cfg = &self.config.model;
        if !valid_file_name(&file) {
            return Err(anyhow!("'{}' is not an acceptable model file name", file));
        }
        let catalog = load_catalog(&cfg.catalog);
        if !allowed_url(&url, &catalog) {
            return Err(anyhow!("downloads are limited to https://huggingface.co (or the catalog)"));
        }
        {
            let mut cur = self.download.lock().unwrap();
            if cur.as_ref().map(|d| !d.done).unwrap_or(false) {
                return Err(anyhow!("another download is still running"));
            }
            *cur = Some(DownloadState { file: file.clone(), url: url.clone(), received: 0, total: size, done: false, error: None });
        }
        self.download_cancel.store(false, Ordering::Relaxed);
        std::fs::create_dir_all(&cfg.models_dir).ok();
        let dest = cfg.models_dir.join(&file);
        let part = cfg.models_dir.join(format!("{file}.part"));
        let d = self.clone();
        tokio::spawn(async move {
            let result = run_curl(&d, &url, &part, size, &events).await;
            let error = match result {
                Ok(()) => match std::fs::rename(&part, &dest) {
                    Ok(()) => None,
                    Err(e) => Some(format!("cannot move the finished file into place: {e}")),
                },
                Err(e) => {
                    let _ = std::fs::remove_file(&part);
                    Some(format!("{e:#}"))
                }
            };
            let state = {
                let mut cur = d.download.lock().unwrap();
                if let Some(s) = cur.as_mut() {
                    s.done = true;
                    s.error = error.clone();
                    if error.is_none() {
                        s.received = s.total.max(s.received);
                    }
                }
                cur.clone()
            };
            if let Some(s) = state {
                let _ = events.send(Event::Download(s));
            }
            if error.is_none() && use_after {
                if let Err(e) = d.set_model(&dest.to_string_lossy()) {
                    eprintln!("mindd: cannot switch to the downloaded model: {e:#}");
                }
                let _ = events.send(d.models_event());
            }
        });
        Ok(())
    }

    pub fn cancel_download(&self) {
        self.download_cancel.store(true, Ordering::Relaxed);
    }
}

fn same_file(a: &Path, b: &Path) -> bool {
    match (std::fs::canonicalize(a), std::fs::canonicalize(b)) {
        (Ok(x), Ok(y)) => x == y,
        _ => a == b,
    }
}

async fn run_curl(d: &Daemon, url: &str, part: &Path, total_hint: u64, events: &tokio::sync::mpsc::UnboundedSender<Event>) -> Result<()> {
    let mut child = tokio::process::Command::new("curl")
        .args(["-fsSL", "--retry", "3", "--retry-delay", "2", "-C", "-", "-o"])
        .arg(part)
        .arg(url)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .context("starting curl")?;
    let mut stderr = child.stderr.take();
    let mut total = total_hint;
    if total == 0 {
        total = content_length(url).await.unwrap_or(0);
    }
    loop {
        if d.download_cancel.load(Ordering::Relaxed) {
            let _ = child.kill().await;
            return Err(anyhow!("cancelled"));
        }
        let received = std::fs::metadata(part).map(|m| m.len()).unwrap_or(0);
        let snapshot = {
            let mut cur = d.download.lock().unwrap();
            if let Some(s) = cur.as_mut() {
                s.received = received;
                s.total = total;
            }
            cur.clone()
        };
        if let Some(s) = snapshot {
            let _ = events.send(Event::Download(s));
        }
        match tokio::time::timeout(Duration::from_millis(700), child.wait()).await {
            Ok(status) => {
                let status = status.context("waiting for curl")?;
                if status.success() {
                    return Ok(());
                }
                let mut msg = String::new();
                if let Some(mut e) = stderr.take() {
                    use tokio::io::AsyncReadExt;
                    let _ = e.read_to_string(&mut msg).await;
                }
                let msg = msg.lines().last().unwrap_or("").trim().to_string();
                return Err(anyhow!("download failed ({}){}", status, if msg.is_empty() { String::new() } else { format!(": {msg}") }));
            }
            Err(_) => continue,
        }
    }
}

/// `Content-Length` after redirects, for the progress bar.
async fn content_length(url: &str) -> Option<u64> {
    let out = tokio::process::Command::new("curl")
        .args(["-sIL", "--max-time", "20", url])
        .output()
        .await
        .ok()?;
    let text = String::from_utf8_lossy(&out.stdout);
    let mut best = None;
    for line in text.lines() {
        if let Some(v) = line.strip_prefix("content-length:").or_else(|| line.strip_prefix("Content-Length:")) {
            if let Ok(n) = v.trim().parse::<u64>() {
                best = Some(n); // the last response in the redirect chain wins
            }
        }
    }
    best
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_names_and_urls_are_checked() {
        assert!(valid_file_name("Qwen3.5-4B-Q4_K_M.gguf"));
        assert!(!valid_file_name("default.gguf"));
        assert!(!valid_file_name("../x.gguf"));
        assert!(!valid_file_name("x.bin"));
        assert!(!valid_file_name(".hidden.gguf"));
        assert!(allowed_url("https://huggingface.co/unsloth/Qwen3.5-4B-GGUF/resolve/main/x.gguf", &[]));
        assert!(allowed_url("https://cdn-lfs-us-1.huggingface.co/repos/x", &[]));
        assert!(!allowed_url("http://huggingface.co/x", &[]));
        assert!(!allowed_url("https://example.com/x.gguf", &[]));
        let cat = vec![CatalogEntry { id: "x".into(), name: "x".into(), file: "x.gguf".into(), url: "https://example.com/x.gguf".into(), size: 1, license: String::new(), license_url: String::new(), params: String::new(), description: String::new(), min_vram_gb: 0.0, recommended: false, installed: false }];
        assert!(allowed_url("https://example.com/x.gguf", &cat));
    }

    #[test]
    fn prefs_round_trip() {
        let dir = std::env::temp_dir().join(format!("mind-prefs-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        assert_eq!(load_prefs(&dir).thinking, None);
        save_prefs(&dir, &Prefs { thinking: Some(true), auto_update: None }).unwrap();
        assert_eq!(load_prefs(&dir).thinking, Some(true));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn default_link_is_followed() {
        let dir = std::env::temp_dir().join(format!("mind-models-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let cfg = ModelConfig { models_dir: dir.clone(), ..ModelConfig::default() };
        std::fs::write(dir.join("a.gguf"), b"x").unwrap();
        std::fs::write(dir.join("b.gguf"), b"xy").unwrap();
        assert_eq!(default_target(&cfg), None);
        std::os::unix::fs::symlink(dir.join("b.gguf"), dir.join("default.gguf")).unwrap();
        assert_eq!(default_target(&cfg).unwrap().file_name().unwrap(), "b.gguf");
        let list = installed(&cfg);
        assert_eq!(list.iter().map(|m| m.file.as_str()).collect::<Vec<_>>(), vec!["a.gguf", "b.gguf"]);
        // a dangling default link is ignored, both here and by select_model
        std::fs::remove_file(dir.join("b.gguf")).unwrap();
        assert_eq!(default_target(&cfg), None);
        assert_eq!(super::super::llm::select_model(&cfg).unwrap().file_name().unwrap(), "a.gguf");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
