//! The agent loop: model → tool calls → confirmation → results → model.

use super::server::Conn;
use super::{llm, policy, tools, Daemon};
use crate::proto::{Event, Policy, Request};
use anyhow::{anyhow, Result};
use serde_json::{json, Value};
use std::time::Duration;

pub struct Session {
    pub id: String,
    pub messages: Vec<llm::Message>,
    pub created: std::time::Instant,
}

pub fn system_prompt() -> String {
    let facts = super::sysinfo::summary_text();
    format!(
        "You are Mind, the operator of this MindOS computer. MindOS is a gaming-first Linux distribution \
(Arch-based, pacman, systemd) with its own kernel (linux-mindos) and its own compositor (mindwm). \
You administer the machine for its user through tools: updating, installing packages and drivers, \
managing services, reading logs, diagnosing hardware and configuring the system.

Rules:
- Use tools to look before you act; never guess package names or file contents.
- Prefer the dedicated tools (check_updates, install_packages, journal, ...) over run_command.
- Changes to the system are shown to the user for confirmation. If a change is declined, do not retry it; \
explain what you would have done instead.
- Be concise and concrete. Report what you did and what you found. Use plain sentences, short lists when listing.
- Do not invent results. If a tool fails, say so and suggest the next step.
- Packages come from the MindOS and Arch repositories, from Flathub (flatpak) and from the AUR (built \
locally). install_packages tries them in that order and says which one it used; a not-found answer from it means \
the name exists in none of them, so suggest search_packages, never 'install it first'.
- Package names are plain lower-case names (discord, octopi, steam), never descriptions or hardware names.
- For updates: check Arch news for manual interventions first, apply the update, then report pacnew files \
and whether a reboot is needed (new kernel or NVIDIA driver).
- Never touch the mind's own audit log, never disable mindd, never format disks.
- You also look after the user: the update watcher (update_status) rates pending updates and posts notices; the \
health checks (health_check) verify the system after every update; snapshots (list_snapshots, rollback) are the way \
back from a bad update, and every pacman run is bracketed by them. When something looks broken after an update, say which \
package is the likely cause and offer the rollback, explaining that the current state is kept so it can be undone.
- Performance: performance_mode switches balanced/performance/quiet; GameMode already goes to performance while a game runs \
and unloads you from the GPU meanwhile (mind_sleep). Suggest performance mode for benchmarks or a stuttering game, quiet for \
a warm room or a laptop on battery.
- Games: the dlss tool swaps DLSS / FSR / XeSS DLLs per game with a backup; suggest it when a game's DLSS is old \
(Super Resolution 310.x is current) or the user asks about upscaling quality. Proton 10+ also honours PROTON_DLSS_UPGRADE=1 \
in a game's launch options.
- Developers: the mindos-dev stack (Rust, Node, Python, Go, Docker/Podman, distrobox, lazygit, delta, starship) is installed \
or one `install_packages mindos-dev` away; `mindos-dev-setup` finishes the per-user setup.
- Guide, do not lecture: one clear recommendation, the reason in a sentence, then act (with confirmation) or stop.

This machine:
{facts}",
        facts = facts
    )
}

/// A short "right now" block refreshed on every turn: performance mode,
/// notices, pending updates, sleep state. It is folded into the one system
/// message (chat templates such as Qwen's reject a system message anywhere
/// but first), after the fixed prompt.
pub fn refresh_status(d: &Daemon, session: &mut Session) {
    let perf = std::fs::read_to_string("/run/mindos/perf/effective").map(|s| s.trim().to_string()).unwrap_or_else(|_| "balanced".into());
    let game = std::fs::read_to_string("/run/mindos/perf/game").map(|s| s.trim() != "0" && !s.trim().is_empty()).unwrap_or(false);
    let u = d.updates.lock().unwrap().clone();
    let updates = if u.packages.is_empty() {
        if u.checked_at == 0 { "not checked yet".to_string() } else { "none pending".to_string() }
    } else {
        format!("{} pending, risk {}{}", u.packages.len(), if u.risk.is_empty() { "unknown" } else { &u.risk }, if u.manual_intervention { ", MANUAL INTERVENTION announced" } else { "" })
    };
    let last = u.last_update.as_ref().map(|l| format!("{} ({} packages, verification: {})", super::health::date(l.time), l.packages.len(), if l.verified.is_empty() { "pending" } else { &l.verified })).unwrap_or_else(|| "none recorded".into());
    let text = format!(
        "Right now ({}):\n- performance mode: {}{}\n- updates: {}\n- last update: {}\n- notices shown to the user:\n{}",
        chrono::Local::now().format("%a %d %b %H:%M"),
        perf,
        if game { " (a game is running)" } else { "" },
        updates,
        last,
        d.notices.summary_text()
    );
    let content = format!("{}\n\n{}", d.system_prompt, text);
    match session.messages.first_mut() {
        Some(m) if m.role == "system" => m.content = Some(content),
        _ => session.messages.insert(0, llm::Message::system(content)),
    }
    // Sessions saved by an older daemon carried the block as a second system message.
    if session.messages.get(1).map(|m| m.role == "system" && m.name.as_deref() == Some("status")).unwrap_or(false) {
        session.messages.remove(1);
    }
}

fn tool_result_summary(v: &Value) -> String {
    let s = match v {
        Value::Object(m) => m.get("output").or_else(|| m.get("content")).and_then(|x| x.as_str()).map(|s| s.to_string()).unwrap_or_else(|| v.to_string()),
        v => v.to_string(),
    };
    let first: String = s.lines().take(3).collect::<Vec<_>>().join(" | ");
    if first.chars().count() > 200 {
        format!("{}…", first.chars().take(197).collect::<String>())
    } else {
        first
    }
}

/// Run one user turn. Streams events to `conn`; returns the final text.
pub async fn run_chat(d: &Daemon, conn: &mut Conn, session: &mut Session, text: String, autopilot: bool) -> Result<String> {
    let defs = tools::definitions();
    let tool_defs = tools::openai_tools(&defs, &conn.client_tools);
    session.messages.push(llm::Message::user(text.clone()));
    d.audit.record("user", &session.id, conn.uid, json!({"text": text, "client": conn.client}));

    let mut final_text = String::new();
    for round in 0..=d.config.daemon.max_tool_rounds {
        if round == d.config.daemon.max_tool_rounds {
            final_text.push_str("\n(stopping: too many tool calls in one turn)");
            break;
        }
        let tx = conn.tx.clone();
        let cancel_rx = &mut conn.rx;
        let mut cancelled = false;
        let (text, calls) = {
            let on_delta = |delta: llm::Delta| {
                let ev = match delta {
                    llm::Delta::Text(t) => Event::Delta { text: t, kind: "text".into() },
                    llm::Delta::Thinking(t) => Event::Delta { text: t, kind: "thinking".into() },
                };
                let _ = tx.send(ev);
            };
            let check_cancel = || {
                // drain any control messages that arrived during streaming
                while let Ok(req) = cancel_rx.try_recv() {
                    if matches!(req, Request::Cancel) {
                        cancelled = true;
                    }
                }
                cancelled
            };
            d.llm.chat(&session.messages, &tool_defs, on_delta, check_cancel).await?
        };
        if cancelled {
            return Err(anyhow!("cancelled"));
        }
        session.messages.push(llm::Message::assistant(&text, &calls));
        if !text.is_empty() {
            d.audit.record("assistant", &session.id, conn.uid, json!({"text": text}));
        }
        if calls.is_empty() {
            final_text = text;
            break;
        }
        for call in calls {
            let args: Value = serde_json::from_str(&call.arguments).unwrap_or_else(|_| json!({"_raw": call.arguments}));
            let result = run_tool(d, conn, session, &defs, &call, &args, autopilot).await;
            let content = match result {
                Ok(v) => v.to_string(),
                Err(e) => json!({"ok": false, "error": e.to_string()}).to_string(),
            };
            session.messages.push(llm::Message::tool(&call.id, &call.name, content));
        }
    }
    d.audit.record("done", &session.id, conn.uid, json!({"text": final_text}));
    Ok(final_text)
}

async fn run_tool(d: &Daemon, conn: &mut Conn, session: &Session, defs: &[tools::ToolDef], call: &llm::ToolCall, args: &Value, autopilot: bool) -> Result<Value> {
    // client-side tool?
    if let Some(ct) = conn.client_tools.iter().find(|t| t.name == call.name).cloned() {
        conn.send(Event::ToolCall { id: call.id.clone(), name: call.name.clone(), args: args.clone(), policy: Policy::Change, needs_confirmation: false });
        conn.send(Event::ClientTool { id: call.id.clone(), name: ct.name.clone(), args: args.clone() });
        d.audit.record("client_tool", &session.id, conn.uid, json!({"name": call.name, "args": args}));
        let res = conn.wait_tool_result(&call.id, Duration::from_secs(120)).await;
        return match res {
            Some((ok, v)) => {
                conn.send(Event::ToolResult { id: call.id.clone(), name: call.name.clone(), ok, summary: tool_result_summary(&v) });
                Ok(json!({"ok": ok, "result": v}))
            }
            None => Err(anyhow!("the client did not answer the tool request")),
        };
    }
    let def = tools::find(defs, &call.name).ok_or_else(|| anyhow!("unknown tool {}", call.name))?;
    let pol = tools::policy_for(def, args, &d.config);
    let needs = policy::needs_confirmation(pol, def.category, autopilot, &d.config.policy);
    conn.send(Event::ToolCall { id: call.id.clone(), name: call.name.clone(), args: args.clone(), policy: pol, needs_confirmation: needs && pol != Policy::Forbidden });
    d.audit.record("tool_call", &session.id, conn.uid, json!({"name": call.name, "args": args, "policy": pol, "needs_confirmation": needs}));
    if pol == Policy::Forbidden {
        conn.send(Event::ToolResult { id: call.id.clone(), name: call.name.clone(), ok: false, summary: "forbidden by policy".into() });
        d.audit.record("tool_denied", &session.id, conn.uid, json!({"name": call.name, "reason": "forbidden"}));
        return Err(anyhow!("this action is forbidden by MindOS policy"));
    }
    if needs {
        let approved = conn.wait_confirm(&call.id, Duration::from_secs(600)).await;
        if !approved {
            conn.send(Event::ToolResult { id: call.id.clone(), name: call.name.clone(), ok: false, summary: "declined by user".into() });
            d.audit.record("tool_denied", &session.id, conn.uid, json!({"name": call.name, "reason": "declined"}));
            return Err(anyhow!("the user declined this action"));
        }
    }
    let started = std::time::Instant::now();
    let result = tools::execute(&call.name, args, &d.config, d, &tools::Caller { uid: conn.uid }).await;
    let elapsed = started.elapsed().as_secs_f64();
    match &result {
        Ok(v) => {
            let ok = v.get("ok").and_then(|b| b.as_bool()).unwrap_or(true);
            conn.send(Event::ToolResult { id: call.id.clone(), name: call.name.clone(), ok, summary: tool_result_summary(v) });
            d.audit.record("tool_result", &session.id, conn.uid, json!({"name": call.name, "ok": ok, "secs": elapsed, "summary": tool_result_summary(v)}));
        }
        Err(e) => {
            conn.send(Event::ToolResult { id: call.id.clone(), name: call.name.clone(), ok: false, summary: e.to_string() });
            d.audit.record("tool_result", &session.id, conn.uid, json!({"name": call.name, "ok": false, "secs": elapsed, "error": e.to_string()}));
        }
    }
    result
}
