//! The daemon: model process management, the agent loop, tools, policy, audit.

pub mod agent;
pub mod audit;
pub mod llm;
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
    pub sessions: tokio::sync::Mutex<std::collections::HashMap<String, agent::Session>>,
    pub system_prompt: String,
}

impl Daemon {
    pub fn new(config: Config, llm: llm::Llm, audit: audit::Audit) -> Arc<Daemon> {
        let system_prompt = agent::system_prompt();
        Arc::new(Daemon {
            config,
            llm,
            audit,
            ready: AtomicBool::new(false),
            started: Instant::now(),
            model_name: std::sync::Mutex::new(String::from("(loading)")),
            sessions: tokio::sync::Mutex::new(Default::default()),
            system_prompt,
        })
    }
    pub fn is_ready(&self) -> bool {
        self.ready.load(Ordering::Acquire)
    }
    pub fn model_name(&self) -> String {
        self.model_name.lock().unwrap().clone()
    }
}
