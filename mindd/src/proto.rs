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
    Error { message: String },
}

fn default_kind() -> String {
    "text".into()
}
