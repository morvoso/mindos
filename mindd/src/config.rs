//! /etc/mindos/mind.toml

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub model: ModelConfig,
    pub daemon: DaemonConfig,
    pub policy: PolicyConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ModelConfig {
    /// Path to a GGUF file, or "auto" to pick the best model in `models_dir`.
    pub path: String,
    pub models_dir: PathBuf,
    pub context: u32,
    /// -1 = everything the GPU can take, 0 = CPU only.
    pub gpu_layers: i32,
    pub threads: u32,
    pub llama_server: PathBuf,
    pub host: String,
    pub port: u16,
    /// Extra arguments for llama-server.
    pub extra_args: Vec<String>,
    /// Use an already running OpenAI-compatible server instead of spawning one.
    pub external_url: Option<String>,
    pub temperature: f32,
    pub max_tokens: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct DaemonConfig {
    pub socket: PathBuf,
    pub log: PathBuf,
    pub state_dir: PathBuf,
    /// Group that may talk to the daemon (the socket is chmod 660 root:group).
    pub group: String,
    pub max_tool_rounds: usize,
    pub tool_timeout_secs: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct PolicyConfig {
    /// Run every "change" tool without confirmation.
    pub autopilot: bool,
    /// Tool categories that never need confirmation (e.g. "update").
    pub autopilot_categories: Vec<String>,
    /// Extra read-only commands allowed through run_command without confirmation.
    pub extra_observe_commands: Vec<String>,
}

impl Default for Config {
    fn default() -> Self {
        Config { model: ModelConfig::default(), daemon: DaemonConfig::default(), policy: PolicyConfig::default() }
    }
}

impl Default for ModelConfig {
    fn default() -> Self {
        ModelConfig {
            path: "auto".into(),
            models_dir: PathBuf::from("/var/lib/mindos/models"),
            context: 8192,
            gpu_layers: -1,
            threads: 0,
            llama_server: PathBuf::from("/usr/bin/llama-server"),
            host: "127.0.0.1".into(),
            port: 8642,
            extra_args: vec![],
            external_url: None,
            temperature: 0.3,
            max_tokens: 2048,
        }
    }
}

impl Default for DaemonConfig {
    fn default() -> Self {
        DaemonConfig {
            socket: PathBuf::from("/run/mindos/mind.sock"),
            log: PathBuf::from("/var/log/mindos/mind.jsonl"),
            state_dir: PathBuf::from("/var/lib/mindos"),
            group: "mindos".into(),
            max_tool_rounds: 16,
            tool_timeout_secs: 1800,
        }
    }
}

impl Default for PolicyConfig {
    fn default() -> Self {
        PolicyConfig { autopilot: false, autopilot_categories: vec![], extra_observe_commands: vec![] }
    }
}

impl Config {
    pub fn load(path: Option<&Path>) -> anyhow::Result<Config> {
        let path = match path {
            Some(p) => p.to_path_buf(),
            None => std::env::var_os("MIND_CONFIG").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("/etc/mindos/mind.toml")),
        };
        let mut cfg = if path.exists() {
            let s = std::fs::read_to_string(&path)?;
            toml::from_str(&s).map_err(|e| anyhow::anyhow!("{}: {}", path.display(), e))?
        } else {
            Config::default()
        };
        if let Some(s) = std::env::var_os("MIND_SOCKET") {
            cfg.daemon.socket = PathBuf::from(s);
        }
        if let Some(s) = std::env::var_os("MIND_MODEL") {
            cfg.model.path = s.to_string_lossy().into_owned();
        }
        if let Some(s) = std::env::var_os("MIND_LOG") {
            cfg.daemon.log = PathBuf::from(s);
        }
        if let Some(s) = std::env::var_os("MIND_LLM_URL") {
            cfg.model.external_url = Some(s.to_string_lossy().into_owned());
        }
        Ok(cfg)
    }

    pub fn socket_path() -> PathBuf {
        std::env::var_os("MIND_SOCKET").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("/run/mindos/mind.sock"))
    }
}
