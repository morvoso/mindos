//! Newline-delimited JSON protocol spoken over /run/mindos/mind.sock.
//!
//! Clients (the `mind` CLI, the compositor's Mind bar, the update timer) send
//! `Request`s; the daemon answers with a stream of `Event`s. A `Chat` request
//! produces `Delta`s, `ToolCall`s (which may need a `Confirm`), `ToolResult`s
//! and finally a `Done`.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// A tool the *client* can execute on the daemon's behalf (launching an
/// application inside the user's session, showing a notification, ...).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClientTool {
    pub name: String,
    pub description: String,
    pub parameters: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Request {
    /// Identify the client and register client-side tools.
    Hello {
        client: String,
        #[serde(default)]
        tools: Vec<ClientTool>,
    },
    /// Talk to the mind. `session` continues an earlier conversation.
    Chat {
        #[serde(default)]
        session: Option<String>,
        text: String,
        /// Run "change" tools without asking for confirmation.
        #[serde(default)]
        autopilot: bool,
    },
    /// Answer to a `ToolCall` event that needed confirmation.
    Confirm { id: String, approve: bool },
    /// Result of a `ClientTool` event.
    ToolResult {
        id: String,
        ok: bool,
        #[serde(default)]
        result: Value,
    },
    /// Stop the current chat.
    Cancel,
    Status,
    History {
        #[serde(default = "default_limit")]
        limit: usize,
    },
    /// Installed models, the catalog and the current choice (answered with `Models`).
    Models,
    /// Use `path` (a file in models_dir or an absolute path; "" or "auto" for
    /// the automatic choice) from now on; llama-server restarts.
    SetModel { path: String },
    /// Let the model think before answering (restarts llama-server).
    SetThinking { enabled: bool },
    /// Download `url` into models_dir as `file`; progress arrives as `Download`
    /// events. `use_after` switches to it when it is complete.
    DownloadModel {
        url: String,
        file: String,
        #[serde(default)]
        size: u64,
        #[serde(default)]
        use_after: bool,
    },
    CancelDownload,
}

/// A GGUF file in the models directory.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelEntry {
    pub file: String,
    pub path: String,
    pub size: u64,
    /// The model llama-server is (or will be) running.
    pub active: bool,
}

/// One downloadable model from `/etc/mindos/model-catalog.json`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct CatalogEntry {
    pub id: String,
    pub name: String,
    pub file: String,
    pub url: String,
    pub size: u64,
    pub license: String,
    pub license_url: String,
    pub params: String,
    pub description: String,
    pub min_vram_gb: f64,
    pub recommended: bool,
    /// Filled in by the daemon: the file is already in models_dir.
    pub installed: bool,
}

impl Default for CatalogEntry {
    fn default() -> Self {
        CatalogEntry {
            id: String::new(),
            name: String::new(),
            file: String::new(),
            url: String::new(),
            size: 0,
            license: String::new(),
            license_url: String::new(),
            params: String::new(),
            description: String::new(),
            min_vram_gb: 0.0,
            recommended: false,
            installed: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DownloadState {
    pub file: String,
    pub url: String,
    pub received: u64,
    /// 0 when unknown.
    pub total: u64,
    pub done: bool,
    pub error: Option<String>,
}

fn default_limit() -> usize {
    50
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Policy {
    /// Read-only: runs immediately.
    Observe,
    /// Changes the system: needs confirmation unless autopilot is on.
    Change,
    /// Never runs.
    Forbidden,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Event {
    Welcome {
        version: String,
        model: String,
        ready: bool,
    },
    /// Streamed assistant text. `kind` is "text" or "thinking".
    Delta {
        text: String,
        #[serde(default = "default_kind")]
        kind: String,
    },
    /// The model wants to run a tool. If `needs_confirmation` the daemon waits
    /// for a `Confirm` request with the same id.
    ToolCall {
        id: String,
        name: String,
        args: Value,
        policy: Policy,
        needs_confirmation: bool,
    },
    ToolResult {
        id: String,
        name: String,
        ok: bool,
        summary: String,
    },
    /// The daemon asks the client to run one of its registered tools.
    ClientTool { id: String, name: String, args: Value },
    Done { session: String, text: String },
    Status {
        version: String,
        model: String,
        ready: bool,
        sessions: usize,
        uptime_secs: u64,
        backend: String,
    },
    History { entries: Vec<Value> },
    /// Answer to `Models` (and after `SetModel` / `SetThinking`).
    Models {
        /// Path of the model in use (or about to load), if any.
        current: Option<String>,
        /// Display name (file name) of the running model, "(loading)" meanwhile.
        model: String,
        ready: bool,
        /// No explicit choice: the largest model that fits the GPU is picked.
        auto: bool,
        thinking: bool,
        models_dir: String,
        /// An external server answers; model switching is not available.
        external: bool,
        /// Total memory of the first GPU in bytes, if known.
        gpu_memory: Option<u64>,
        models: Vec<ModelEntry>,
        catalog: Vec<CatalogEntry>,
        /// The running (or last finished) download.
        download: Option<DownloadState>,
    },
    Download(DownloadState),
    Error { message: String },
}

fn default_kind() -> String {
    "text".into()
}
