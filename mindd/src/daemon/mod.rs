//! The daemon: model process management, the agent loop, tools, policy, audit.

pub mod agent;
pub mod audit;
pub mod health;
pub mod llm;
pub mod models;
pub mod notices;
pub mod policy;
pub mod server;
pub mod sysinfo;
pub mod tools;
pub mod updates;

use crate::config::Config;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Instant;

pub struct Daemon {
    pub config: Config,
    pub llm: llm::Llm,
    pub audit: audit::Audit,
    pub ready: AtomicBool,
    pub started: Instant,
    pub model_name: std::sync::Mutex<String>,
    /// Path of the model llama-server is running (None before the first start).
    pub model_path: std::sync::Mutex<Option<std::path::PathBuf>>,
    pub sessions: tokio::sync::Mutex<std::collections::HashMap<String, agent::Session>>,
    pub system_prompt: String,
    /// Effective "think before answering" setting (config, overridden by mind-prefs.json).
    pub thinking: AtomicBool,
    /// Signalled when the model or the thinking setting changed: the
    /// supervisor restarts llama-server.
    pub restart: tokio::sync::Notify,
    pub download: std::sync::Mutex<Option<crate::proto::DownloadState>>,
    pub download_cancel: AtomicBool,
    /// What the user should know (notifications in the shell).
    pub notices: notices::Notices,
    /// The update watcher's state.
    pub updates: std::sync::Mutex<crate::proto::UpdateStatus>,
    /// Signalled to run an update check right away.
    pub check_now: tokio::sync::Notify,
    /// Install low-risk updates without asking (config, overridden by prefs).
    pub auto_update: AtomicBool,
    /// The last health check: when, and what it found.
    pub last_health: std::sync::Mutex<(u64, Vec<crate::proto::Finding>)>,
    /// The model is unloaded on purpose (a game needs the GPU).
    pub sleeping: AtomicBool,
}

impl Daemon {
    pub fn new(config: Config, llm: llm::Llm, audit: audit::Audit) -> Arc<Daemon> {
        let system_prompt = agent::system_prompt();
        let prefs = models::load_prefs(&config.daemon.state_dir);
        let thinking = prefs.thinking.unwrap_or(config.model.thinking);
        let auto_update = prefs.auto_update.unwrap_or(config.updates.auto_apply);
        let notices = notices::Notices::open(&config.daemon.state_dir);
        let mut updates = updates::load(&config.daemon.state_dir);
        updates.auto_apply = auto_update;
        Arc::new(Daemon {
            config,
            llm,
            audit,
            ready: AtomicBool::new(false),
            started: Instant::now(),
            model_name: std::sync::Mutex::new(String::from("(loading)")),
            model_path: std::sync::Mutex::new(None),
            sessions: tokio::sync::Mutex::new(Default::default()),
            system_prompt,
            thinking: AtomicBool::new(thinking),
            restart: tokio::sync::Notify::new(),
            download: std::sync::Mutex::new(None),
            download_cancel: AtomicBool::new(false),
            notices,
            updates: std::sync::Mutex::new(updates),
            check_now: tokio::sync::Notify::new(),
            auto_update: AtomicBool::new(auto_update),
            last_health: std::sync::Mutex::new((0, vec![])),
            sleeping: AtomicBool::new(false),
        })
    }
    pub fn auto_update(&self) -> bool {
        self.auto_update.load(Ordering::Relaxed)
    }
    pub fn set_auto_update(&self, enabled: bool) -> anyhow::Result<()> {
        self.auto_update.store(enabled, Ordering::Relaxed);
        let mut prefs = models::load_prefs(&self.config.daemon.state_dir);
        prefs.auto_update = Some(enabled);
        models::save_prefs(&self.config.daemon.state_dir, &prefs)?;
        self.updates.lock().unwrap().auto_apply = enabled;
        self.notify_updates();
        Ok(())
    }
    /// Push the update status to subscribed clients.
    pub fn notify_updates(&self) {
        let s = self.updates.lock().unwrap().clone();
        self.notices.broadcast(crate::proto::Event::Updates(s));
    }
    pub fn is_sleeping(&self) -> bool {
        self.sleeping.load(Ordering::Acquire)
    }
    /// Unload the model (sleeping) or load it again; the supervisor reacts.
    pub fn set_sleep(&self, sleeping: bool) {
        if self.sleeping.swap(sleeping, Ordering::AcqRel) != sleeping {
            eprintln!("mindd: {}", if sleeping { "going to sleep (model unloaded)" } else { "waking up" });
            self.restart.notify_one();
            self.notices.broadcast(crate::proto::Event::Sleep { sleeping });
        }
    }
    /// Wake the model and wait for it, up to `secs`.
    pub async fn wake_and_wait(&self, secs: u64) -> bool {
        self.set_sleep(false);
        let started = std::time::Instant::now();
        while started.elapsed() < std::time::Duration::from_secs(secs) {
            if self.is_ready() {
                return true;
            }
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        }
        self.is_ready()
    }
    pub fn is_ready(&self) -> bool {
        self.ready.load(Ordering::Acquire)
    }
    pub fn model_name(&self) -> String {
        self.model_name.lock().unwrap().clone()
    }
}
