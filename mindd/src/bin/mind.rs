// Copyright (c) 2026 Black Arrow Software, LLC. All rights reserved.
// Licensed under the MindOS Software License; see LICENSE at the repository root.

//! mind: talk to the MindOS mind from a terminal.

use anyhow::{anyhow, Result};
use clap::{Parser, Subcommand};
use mindos_mind::client::Client;
use mindos_mind::config::Config;
use mindos_mind::proto::{Event, Policy, Request};
use std::io::{IsTerminal, Write};

#[derive(Parser)]
#[command(name = "mind", version, about = "Talk to the MindOS mind")]
struct Cli {
    /// Run changes without asking for confirmation.
    #[arg(short = 'y', long, global = true)]
    autopilot: bool,
    /// Continue a session id printed by an earlier run.
    #[arg(short, long, global = true)]
    session: Option<String>,
    /// Show the model's thinking, when the model produces it.
    #[arg(long, global = true)]
    thinking: bool,
    #[command(subcommand)]
    cmd: Option<Cmd>,
    /// What to say to the mind.
    #[arg(trailing_var_arg = true)]
    text: Vec<String>,
}

#[derive(Subcommand)]
enum Cmd {
    /// Update the system (the mind checks Arch news, applies updates, reports).
    Update,
    /// Diagnose the machine: GPU driver, failed services, errors in the log.
    Doctor,
    /// Daemon and model status.
    Status,
    /// Show the audit log.
    History {
        #[arg(short = 'n', long, default_value_t = 30)]
        limit: usize,
    },
    /// Interactive chat.
    Chat,
    /// Installed models, the download catalog and the current choice.
    Models,
    /// Switch to a model: a file name from `mind models`, a path, or "auto".
    Model { path: String },
    /// Let the model think before answering (slower; off by default).
    Thinking {
        #[arg(value_parser = ["on", "off"])]
        state: String,
    },
    /// Download a catalog model by id (see `mind models`) and switch to it.
    Download {
        id: String,
        /// Only download; keep the current model.
        #[arg(long)]
        keep: bool,
    },
    /// Notices the Mind has for you (update assessments, health findings).
    Notices {
        /// Dismiss a notice by id, or "*" for all.
        #[arg(long)]
        dismiss: Option<String>,
    },
    /// Pending updates and the Mind's risk assessment.
    Updates {
        /// Check the repositories and Arch news now.
        #[arg(long)]
        check: bool,
        /// Install the pending updates (snapshots are taken around it).
        #[arg(long)]
        apply: bool,
        /// Install low-risk updates automatically from now on (on|off).
        #[arg(long, value_parser = ["on", "off"])]
        auto: Option<String>,
    },
    /// Run the health checks: services, kernel, GPU driver, disks, configuration.
    Health,
    /// Unload the model from the GPU (on) or bring it back (off).
    Sleep {
        #[arg(value_parser = ["on", "off"])]
        state: String,
    },
    /// Watch the Mind's notices as they arrive (Ctrl-C to stop).
    Watch,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let mut client = Client::connect(&Config::socket_path()).await?;
    client.send(&Request::Hello { client: "mind-cli".into(), tools: vec![] }).await?;
    match client.next().await? {
        Some(Event::Welcome { ready, model, .. }) => {
            if !ready {
                eprintln!("mind: the model ({}) is still loading; requests will fail until it is ready", model);
            }
        }
        _ => return Err(anyhow!("unexpected reply from mindd")),
    }
    let text = cli.text.join(" ");
    match cli.cmd {
        Some(Cmd::Status) => {
            client.send(&Request::Status).await?;
            if let Some(Event::Status { version, model, ready, sessions, uptime_secs, backend, sleeping, notices }) = client.next().await? {
                println!("mindd {}  model {}  {}  backend {}  sessions {}  up {}s  notices {}", version, model, if sleeping { "sleeping" } else if ready { "ready" } else { "loading" }, backend, sessions, uptime_secs, notices);
            }
        }
        Some(Cmd::History { limit }) => {
            client.send(&Request::History { limit }).await?;
            if let Some(Event::History { entries }) = client.next().await? {
                for e in entries {
                    let ts = e["ts"].as_str().unwrap_or("").chars().take(19).collect::<String>();
                    let kind = e["kind"].as_str().unwrap_or("");
                    let body = match kind {
                        "user" | "assistant" | "done" => e["text"].as_str().unwrap_or("").lines().next().unwrap_or("").to_string(),
                        "tool_call" => format!("{} {}", e["name"].as_str().unwrap_or(""), e["args"]),
                        "tool_result" => format!("{} ok={} {}", e["name"].as_str().unwrap_or(""), e["ok"], e["summary"].as_str().unwrap_or("")),
                        "tool_denied" => format!("{} {}", e["name"].as_str().unwrap_or(""), e["reason"].as_str().unwrap_or("")),
                        _ => e.to_string(),
                    };
                    println!("{} {:<12} {}", ts, kind, body.chars().take(160).collect::<String>());
                }
            }
        }
        Some(Cmd::Models) => {
            client.send(&Request::Models).await?;
            if let Some(Event::Models { current, model, ready, auto, thinking, models_dir, external, gpu_memory, models, catalog, download }) = client.next().await? {
                println!("running: {} ({}){}", model, if ready { "ready" } else { "loading" }, if external { "  [external server]" } else { "" });
                println!("choice:  {}", if auto { "automatic (largest model that fits the GPU)".to_string() } else { current.unwrap_or_default() });
                println!("thinking: {}   gpu memory: {}", if thinking { "on" } else { "off" }, gpu_memory.map(|b| format!("{:.1} GiB", b as f64 / 1073741824.0)).unwrap_or_else(|| "unknown".into()));
                println!("\ninstalled in {}:", models_dir);
                for m in &models {
                    println!("  {} {:<44} {:>7.2} GiB", if m.active { "*" } else { " " }, m.file, m.size as f64 / 1073741824.0);
                }
                if models.is_empty() {
                    println!("  (none)");
                }
                println!("\ncatalog:");
                for c in &catalog {
                    println!("  {:<14} {:<36} {:>6.2} GiB  {}{}{}", c.id, c.name, c.size as f64 / 1073741824.0, c.license, if c.recommended { "  recommended" } else { "" }, if c.installed { "  [installed]" } else { "" });
                }
                if let Some(dl) = download {
                    println!("\ndownload: {} {}/{} {}", dl.file, dl.received, dl.total, dl.error.map(|e| format!("FAILED: {e}")).unwrap_or_else(|| if dl.done { "done".into() } else { "running".into() }));
                }
            }
        }
        Some(Cmd::Model { path }) => {
            client.send(&Request::SetModel { path }).await?;
            match client.next().await? {
                Some(Event::Models { model, current, auto, .. }) => println!("switching to {} (llama-server restarts){}", if auto { "the automatic choice".to_string() } else { current.unwrap_or(model) }, ""),
                Some(Event::Error { message }) => return Err(anyhow!("{}", message)),
                _ => {}
            }
        }
        Some(Cmd::Thinking { state }) => {
            client.send(&Request::SetThinking { enabled: state == "on" }).await?;
            match client.next().await? {
                Some(Event::Models { thinking, .. }) => println!("thinking {}", if thinking { "on" } else { "off" }),
                Some(Event::Error { message }) => return Err(anyhow!("{}", message)),
                _ => {}
            }
        }
        Some(Cmd::Download { id, keep }) => {
            client.send(&Request::Models).await?;
            let Some(Event::Models { catalog, .. }) = client.next().await? else { return Err(anyhow!("unexpected reply from mindd")) };
            let entry = catalog.iter().find(|c| c.id == id || c.file == id).ok_or_else(|| anyhow!("no catalog entry '{}' (see `mind models`)", id))?.clone();
            client.send(&Request::DownloadModel { url: entry.url.clone(), file: entry.file.clone(), size: entry.size, use_after: !keep }).await?;
            eprintln!("downloading {} ({:.2} GiB, {})", entry.file, entry.size as f64 / 1073741824.0, entry.license);
            loop {
                match client.next().await? {
                    Some(Event::Download(dl)) => {
                        if let Some(e) = dl.error {
                            eprintln!();
                            return Err(anyhow!("download failed: {}", e));
                        }
                        if dl.done {
                            eprintln!("\rdone: {}                         ", dl.file);
                            if keep {
                                break;
                            }
                        } else if dl.total > 0 {
                            eprint!("\r{:>5.1}%  {:.2} / {:.2} GiB   ", dl.received as f64 * 100.0 / dl.total as f64, dl.received as f64 / 1073741824.0, dl.total as f64 / 1073741824.0);
                        } else {
                            eprint!("\r{:.2} GiB   ", dl.received as f64 / 1073741824.0);
                        }
                    }
                    Some(Event::Models { model, .. }) => {
                        println!("switching to {} (llama-server restarts)", model);
                        break;
                    }
                    Some(Event::Error { message }) => return Err(anyhow!("{}", message)),
                    Some(_) => {}
                    None => return Err(anyhow!("mindd closed the connection")),
                }
            }
        }
        Some(Cmd::Notices { dismiss }) => {
            match dismiss {
                Some(id) => client.send(&Request::DismissNotice { id }).await?,
                None => client.send(&Request::Notices).await?,
            }
            if let Some(Event::Notices { notices }) = client.next().await? {
                if notices.is_empty() {
                    println!("no notices");
                }
                for n in notices {
                    let when = chrono::DateTime::from_timestamp(n.time as i64, 0).map(|d| d.with_timezone(&chrono::Local).format("%d %b %H:%M").to_string()).unwrap_or_default();
                    println!("\x1b[1m[{}] {}\x1b[0m  {}  ({})", n.level, n.title, when, n.id);
                    for line in n.body.lines() {
                        println!("    {}", line);
                    }
                    if !n.actions.is_empty() {
                        println!("    actions: {}", n.actions.iter().map(|a| a.label.clone()).collect::<Vec<_>>().join(", "));
                    }
                }
            }
        }
        Some(Cmd::Updates { check, apply, auto }) => {
            if let Some(a) = auto {
                client.send(&Request::SetAutoUpdate { enabled: a == "on" }).await?;
                let _ = client.next().await?;
                println!("automatic low-risk updates {}", a);
            }
            if apply {
                client.send(&Request::ApplyUpdates).await?;
                eprintln!("updating (pacman -Syu)…");
            } else {
                client.send(&Request::Updates { check }).await?;
                if check {
                    eprintln!("checking…");
                }
            }
            loop {
                match client.next().await? {
                    Some(Event::Updates(s)) => {
                        if s.checking || s.assessing || s.applying {
                            continue;
                        }
                        print_updates(&s);
                        break;
                    }
                    Some(Event::Error { message }) => return Err(anyhow!("{}", message)),
                    Some(_) => {}
                    None => return Err(anyhow!("mindd closed the connection")),
                }
            }
        }
        Some(Cmd::Health) => {
            client.send(&Request::Health).await?;
            if let Some(Event::Health { findings, .. }) = client.next().await? {
                if findings.is_empty() {
                    println!("\x1b[32mall good\x1b[0m: services, kernel, GPU driver, disks and configuration look fine");
                }
                for f in findings {
                    println!("\x1b[1m[{}] {}\x1b[0m", f.level, f.title);
                    for line in f.body.lines() {
                        println!("    {}", line);
                    }
                }
            }
        }
        Some(Cmd::Sleep { state }) => {
            client.send(&Request::SetSleep { sleeping: state == "on" }).await?;
            if let Some(Event::Sleep { sleeping }) = client.next().await? {
                println!("mind {}", if sleeping { "sleeping (model unloaded)" } else { "awake" });
            }
        }
        Some(Cmd::Watch) => {
            client.send(&Request::Subscribe).await?;
            loop {
                match client.next().await? {
                    Some(Event::Notice(n)) => println!("[{}] {} — {}", n.level, n.title, n.body.lines().next().unwrap_or("")),
                    Some(Event::NoticeGone { id }) => println!("(dismissed {})", id),
                    Some(Event::Notices { notices }) => println!("{} notice(s)", notices.len()),
                    Some(Event::Updates(s)) => println!("(updates: {} pending, risk {}{})", s.packages.len(), s.risk, if s.checking { ", checking" } else if s.assessing { ", assessing" } else if s.applying { ", applying" } else { "" }),
                    Some(Event::Sleep { sleeping }) => println!("(mind {})", if sleeping { "asleep" } else { "awake" }),
                    Some(_) => {}
                    None => break,
                }
            }
        }
        Some(Cmd::Update) => {
            chat(&mut client, "Update this system. First check the Arch news for required manual interventions, then check what would be updated, apply the updates, and report: what changed, any .pacnew files to merge, and whether a reboot is needed.".into(), cli.session, true, cli.thinking).await?;
        }
        Some(Cmd::Doctor) => {
            chat(&mut client, "Give this machine a health check: GPU and driver status (is the right driver loaded, is Vulkan working), failed systemd units, errors in the current boot's journal, disk space, and pending updates. Report problems with a suggested fix for each, and say clearly if everything is fine.".into(), cli.session, cli.autopilot, cli.thinking).await?;
        }
        Some(Cmd::Chat) => {
            let mut session = cli.session.clone();
            let stdin = std::io::stdin();
            loop {
                eprint!("\x1b[1mmind>\x1b[0m ");
                let mut line = String::new();
                if stdin.read_line(&mut line)? == 0 {
                    break;
                }
                let line = line.trim().to_string();
                if line.is_empty() {
                    continue;
                }
                if line == "exit" || line == "quit" {
                    break;
                }
                match chat(&mut client, line, session.clone(), cli.autopilot, cli.thinking).await {
                    Ok(s) => session = Some(s),
                    Err(e) => eprintln!("mind: {:#}", e),
                }
            }
        }
        None => {
            if text.trim().is_empty() {
                eprintln!("usage: mind <what you want> | mind update | mind updates [--check|--apply|--auto on|off] | mind health | mind notices | mind doctor | mind status | mind history | mind chat | mind models | mind model <file|auto> | mind download <id> | mind thinking on|off | mind sleep on|off | mind watch");
                return Ok(());
            }
            chat(&mut client, text, cli.session, cli.autopilot, cli.thinking).await?;
        }
    }
    Ok(())
}

/// Run one exchange, printing the stream. Returns the session id.
async fn chat(client: &mut Client, text: String, session: Option<String>, autopilot: bool, show_thinking: bool) -> Result<String> {
    client.send(&Request::Chat { session, text, autopilot }).await?;
    let mut out = std::io::stdout();
    let interactive = std::io::stdin().is_terminal();
    let mut printed = false;
    let mut in_thinking = false;
    loop {
        let Some(ev) = client.next().await? else { return Err(anyhow!("mindd closed the connection")) };
        match ev {
            Event::Delta { text, kind } => {
                if kind == "thinking" {
                    if show_thinking {
                        if !in_thinking {
                            eprint!("\x1b[2m");
                            in_thinking = true;
                        }
                        eprint!("{}", text);
                    }
                } else {
                    if in_thinking {
                        eprint!("\x1b[0m\n");
                        in_thinking = false;
                    }
                    print!("{}", text);
                    printed = true;
                    out.flush()?;
                }
            }
            Event::ToolCall { id, name, args, policy, needs_confirmation } => {
                if printed {
                    println!();
                    printed = false;
                }
                let summary = mindos_mind::daemon::policy::args_summary(&args);
                if needs_confirmation {
                    let approve = if interactive {
                        eprint!("\x1b[33m→ {} {}\x1b[0m\n  run this? [y/N] ", name, summary);
                        let mut line = String::new();
                        std::io::stdin().read_line(&mut line)?;
                        matches!(line.trim(), "y" | "Y" | "yes")
                    } else {
                        eprintln!("→ {} {} (declined: not interactive; use -y to allow changes)", name, summary);
                        false
                    };
                    client.send(&Request::Confirm { id, approve }).await?;
                } else {
                    let tag = match policy {
                        Policy::Observe => "",
                        Policy::Change => " (autopilot)",
                        Policy::Forbidden => " (forbidden)",
                    };
                    eprintln!("\x1b[2m→ {} {}{}\x1b[0m", name, summary, tag);
                }
            }
            Event::ToolResult { name, ok, summary, .. } => {
                eprintln!("\x1b[2m← {} {}{}\x1b[0m", name, if ok { "" } else { "FAILED: " }, summary);
            }
            Event::ClientTool { id, name, .. } => {
                client.send(&Request::ToolResult { id, ok: false, result: serde_json::json!({"error": format!("{} is not available from the CLI", name)}) }).await?;
            }
            Event::Done { session, text } => {
                if in_thinking {
                    eprint!("\x1b[0m\n");
                }
                if !printed && !text.is_empty() {
                    print!("{}", text);
                }
                if !text.ends_with('\n') {
                    println!();
                }
                return Ok(session);
            }
            Event::Error { message } => {
                if printed {
                    println!();
                }
                return Err(anyhow!("{}", message));
            }
            Event::Welcome { .. } | Event::Status { .. } | Event::History { .. } | Event::Models { .. } | Event::Download(_) => {}
            Event::Notices { .. } | Event::Notice(_) | Event::NoticeGone { .. } | Event::Updates(_) | Event::Health { .. } | Event::Sleep { .. } | Event::Permissions { .. } => {}
        }
    }
}

fn print_updates(s: &mindos_mind::proto::UpdateStatus) {
    let when = if s.checked_at == 0 { "never".to_string() } else { chrono::DateTime::from_timestamp(s.checked_at as i64, 0).map(|d| d.with_timezone(&chrono::Local).format("%d %b %H:%M").to_string()).unwrap_or_default() };
    if !s.error.is_empty() {
        println!("check failed: {}", s.error);
    }
    println!("checked {}   risk \x1b[1m{}\x1b[0m{}   auto-update {}", when, if s.risk.is_empty() { "unknown" } else { &s.risk }, if s.assessed_by_model { " (Mind)" } else { " (rules)" }, if s.auto_apply { "on" } else { "off" });
    println!("{}", s.summary);
    for w in &s.warnings {
        println!("  ! {}", w);
    }
    if !s.packages.is_empty() {
        println!();
        for p in &s.packages {
            println!("  {:<32} {:>18} -> {:<18} {}", p.name, p.from, p.to, p.tag);
        }
    }
    if let Some(l) = &s.last_update {
        println!("\nlast update: {} packages{}; verification: {}", l.packages.len(), l.pre_snapshot.map(|n| format!(", snapshot {} before it", n)).unwrap_or_default(), if l.verified.is_empty() { "pending" } else { &l.verified });
        if !l.report.is_empty() {
            println!("{}", l.report);
        }
    }
}
