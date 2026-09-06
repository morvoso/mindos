//! The daemon: model process management, the agent loop, tools, policy, audit.

pub mod agent;
pub mod audit;
pub mod llm;
pub mod models;
pub mod policy;
pub mod server;
pub mod sysinfo;
pub mod tools;

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
}

impl Daemon {
    pub fn new(config: Config, llm: llm::Llm, audit: audit::Audit) -> Arc<Daemon> {
        let system_prompt = agent::system_prompt();
        let thinking = models::load_prefs(&config.daemon.state_dir).thinking.unwrap_or(config.model.thinking);
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
        })
    }
    pub fn is_ready(&self) -> bool {
        self.ready.load(Ordering::Acquire)
    }
    pub fn model_name(&self) -> String {
        self.model_name.lock().unwrap().clone()
    }
}
