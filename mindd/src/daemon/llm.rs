//! llama-server process management and the OpenAI-compatible chat API.

use crate::config::ModelConfig;
use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio::process::{Child, Command};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    /// Raw JSON arguments string as produced by the model.
    pub arguments: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub role: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<Value>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

impl Message {
    pub fn system(s: impl Into<String>) -> Message {
        Message { role: "system".into(), content: Some(s.into()), tool_calls: None, tool_call_id: None, name: None }
    }
    pub fn user(s: impl Into<String>) -> Message {
        Message { role: "user".into(), content: Some(s.into()), tool_calls: None, tool_call_id: None, name: None }
    }
    pub fn assistant(text: &str, calls: &[ToolCall]) -> Message {
        let tool_calls = if calls.is_empty() {
            None
        } else {
            Some(
                calls
                    .iter()
                    .map(|c| json!({"id": c.id, "type": "function", "function": {"name": c.name, "arguments": c.arguments}}))
                    .collect(),
            )
        };
        Message { role: "assistant".into(), content: if text.is_empty() { None } else { Some(text.to_string()) }, tool_calls, tool_call_id: None, name: None }
    }
    pub fn tool(id: &str, name: &str, content: String) -> Message {
        Message { role: "tool".into(), content: Some(content), tool_calls: None, tool_call_id: Some(id.to_string()), name: Some(name.to_string()) }
    }
}

pub struct Llm {
    base: String,
    http: reqwest::Client,
    pub temperature: f32,
    pub max_tokens: u32,
}

#[derive(Debug)]
pub enum Delta {
    Text(String),
    Thinking(String),
}

impl Llm {
    pub fn new(base: String, cfg: &ModelConfig) -> Llm {
        Llm {
            base,
            http: reqwest::Client::builder().timeout(Duration::from_secs(600)).build().expect("http client"),
            temperature: cfg.temperature,
            max_tokens: cfg.max_tokens,
        }
    }

    pub async fn health(&self) -> bool {
        match self.http.get(format!("{}/health", self.base)).timeout(Duration::from_secs(3)).send().await {
            Ok(r) => r.status().is_success(),
            Err(_) => false,
        }
    }

    pub async fn model_name(&self) -> Option<String> {
        let v: Value = self.http.get(format!("{}/v1/models", self.base)).send().await.ok()?.json().await.ok()?;
        let id = v["data"][0]["id"].as_str()?;
        Some(Path::new(id).file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| id.to_string()))
    }

    /// Stream one chat completion. `on_delta` receives text as it arrives.
    /// Returns the final text and any tool calls.
    pub async fn chat(
        &self,
        messages: &[Message],
        tools: &[Value],
        mut on_delta: impl FnMut(Delta),
        mut cancelled: impl FnMut() -> bool,
    ) -> Result<(String, Vec<ToolCall>)> {
        let mut body = json!({
            "model": "mindos",
            "messages": messages,
            "stream": true,
            "temperature": self.temperature,
            "max_tokens": self.max_tokens,
        });
        if !tools.is_empty() {
            body["tools"] = Value::Array(tools.to_vec());
            body["tool_choice"] = json!("auto");
        }
        let mut resp = self
            .http
            .post(format!("{}/v1/chat/completions", self.base))
            .json(&body)
            .send()
            .await
            .context("sending request to the model server")?;
        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(anyhow!("model server returned {}: {}", status, text.chars().take(500).collect::<String>()));
        }
        let mut text = String::new();
        let mut calls: Vec<(String, String, String)> = Vec::new(); // (id, name, args) by index
        let mut buf: Vec<u8> = Vec::new();
        loop {
            if cancelled() {
                return Err(anyhow!("cancelled"));
            }
            let chunk = match resp.chunk().await? {
                Some(c) => c,
                None => break,
            };
            buf.extend_from_slice(&chunk);
            // process complete SSE events (separated by blank lines)
            while let Some(pos) = find_event_end(&buf) {
                let event = String::from_utf8_lossy(&buf[..pos]).into_owned();
                buf.drain(..pos + 2);
                for line in event.lines() {
                    let Some(data) = line.strip_prefix("data:") else { continue };
                    let data = data.trim();
                    if data == "[DONE]" {
                        continue;
                    }
                    let v: Value = match serde_json::from_str(data) {
                        Ok(v) => v,
                        Err(_) => continue,
                    };
                    if let Some(err) = v.get("error") {
                        return Err(anyhow!("model server error: {}", err));
                    }
                    let Some(choice) = v["choices"].get(0) else { continue };
                    let delta = &choice["delta"];
                    if let Some(s) = delta["reasoning_content"].as_str() {
                        if !s.is_empty() {
                            on_delta(Delta::Thinking(s.to_string()));
                        }
                    }
                    if let Some(s) = delta["content"].as_str() {
                        if !s.is_empty() {
                            text.push_str(s);
                            on_delta(Delta::Text(s.to_string()));
                        }
                    }
                    if let Some(tcs) = delta["tool_calls"].as_array() {
                        for tc in tcs {
                            let idx = tc["index"].as_u64().unwrap_or(calls.len() as u64) as usize;
                            while calls.len() <= idx {
                                calls.push((String::new(), String::new(), String::new()));
                            }
                            if let Some(id) = tc["id"].as_str() {
                                calls[idx].0 = id.to_string();
                            }
                            if let Some(n) = tc["function"]["name"].as_str() {
                                calls[idx].1.push_str(n);
                            }
                            if let Some(a) = tc["function"]["arguments"].as_str() {
                                calls[idx].2.push_str(a);
                            }
                        }
                    }
                }
            }
        }
        let calls = calls
            .into_iter()
            .enumerate()
            .filter(|(_, c)| !c.1.is_empty())
            .map(|(i, (id, name, args))| ToolCall {
                id: if id.is_empty() { format!("call_{}", i) } else { id },
                name,
                arguments: if args.trim().is_empty() { "{}".into() } else { args },
            })
            .collect();
        Ok((text, calls))
    }
}

fn find_event_end(buf: &[u8]) -> Option<usize> {
    buf.windows(2).position(|w| w == b"\n\n")
}

/// Pick a model file: an explicit path, or the best GGUF in the models dir.
pub fn select_model(cfg: &ModelConfig) -> Result<PathBuf> {
    if cfg.path != "auto" {
        let p = PathBuf::from(&cfg.path);
        if p.exists() {
            return Ok(p);
        }
        return Err(anyhow!("model file {} does not exist", p.display()));
    }
    let mut candidates: Vec<(u64, PathBuf)> = Vec::new();
    if let Ok(rd) = std::fs::read_dir(&cfg.models_dir) {
        for e in rd.flatten() {
            let p = e.path();
            if p.extension().map(|x| x == "gguf").unwrap_or(false) {
                if let Ok(m) = e.metadata() {
                    candidates.push((m.len(), p));
                }
            }
        }
    }
    if candidates.is_empty() {
        return Err(anyhow!("no .gguf model found in {}", cfg.models_dir.display()));
    }
    // a file called default.gguf (or a symlink) wins
    if let Some((_, p)) = candidates.iter().find(|(_, p)| p.file_name().map(|n| n == "default.gguf").unwrap_or(false)) {
        return Ok(p.clone());
    }
    // otherwise the largest model that fits in VRAM (with room for the KV cache), else the smallest
    let budget = gpu_memory_bytes().map(|b| b * 8 / 10);
    candidates.sort_by_key(|(size, _)| *size);
    let pick = match budget {
        Some(b) => candidates.iter().rev().find(|(size, _)| *size <= b).or(candidates.first()),
        None => candidates.first(),
    };
    Ok(pick.unwrap().1.clone())
}

/// Total memory of the first GPU, if we can find out.
pub fn gpu_memory_bytes() -> Option<u64> {
    // NVIDIA
    if let Ok(out) = std::process::Command::new("nvidia-smi").args(["--query-gpu=memory.total", "--format=csv,noheader,nounits"]).output() {
        if out.status.success() {
            if let Some(line) = String::from_utf8_lossy(&out.stdout).lines().next() {
                if let Ok(mib) = line.trim().parse::<u64>() {
                    return Some(mib * 1024 * 1024);
                }
            }
        }
    }
    // AMD/Intel via sysfs
    if let Ok(rd) = std::fs::read_dir("/sys/class/drm") {
        for e in rd.flatten() {
            let p = e.path().join("device/mem_info_vram_total");
            if let Ok(s) = std::fs::read_to_string(&p) {
                if let Ok(b) = s.trim().parse::<u64>() {
                    return Some(b);
                }
            }
        }
    }
    None
}

pub struct ServerProcess {
    pub child: Child,
    pub gpu: bool,
}

/// Spawn llama-server for `model`. `gpu_layers` follows the config (-1 = all).
pub fn spawn_server(cfg: &ModelConfig, model: &Path, gpu: bool) -> Result<ServerProcess> {
    let mut cmd = Command::new(&cfg.llama_server);
    cmd.arg("-m")
        .arg(model)
        .arg("--host")
        .arg(&cfg.host)
        .arg("--port")
        .arg(cfg.port.to_string())
        .arg("-c")
        .arg(cfg.context.to_string())
        .arg("--jinja")
        .arg("--no-webui")
        .arg("-np")
        .arg("1")
        .arg("--reasoning-format")
        .arg("deepseek")
        .arg("--log-prefix");
    if gpu {
        let layers = if cfg.gpu_layers < 0 { 999 } else { cfg.gpu_layers };
        cmd.arg("-ngl").arg(layers.to_string()).arg("-fa").arg("on");
    } else {
        cmd.arg("-ngl").arg("0");
    }
    if cfg.threads > 0 {
        cmd.arg("-t").arg(cfg.threads.to_string());
    }
    for a in &cfg.extra_args {
        cmd.arg(a);
    }
    cmd.stdout(std::process::Stdio::null()).stderr(std::process::Stdio::inherit()).kill_on_drop(true);
    let child = cmd.spawn().with_context(|| format!("starting {}", cfg.llama_server.display()))?;
    Ok(ServerProcess { child, gpu })
}
