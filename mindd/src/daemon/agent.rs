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
- For updates: check Arch news for manual interventions first, apply the update, then report pacnew files \
and whether a reboot is needed (new kernel or NVIDIA driver).
- Never touch the mind's own audit log, never disable mindd, never format disks.

This machine:
{facts}",
        facts = facts
    )
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
    let result = tools::execute(&call.name, args, &d.config).await;
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
