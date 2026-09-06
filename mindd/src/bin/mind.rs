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
            if let Some(Event::Status { version, model, ready, sessions, uptime_secs, backend }) = client.next().await? {
                println!("mindd {}  model {}  {}  backend {}  sessions {}  up {}s", version, model, if ready { "ready" } else { "loading" }, backend, sessions, uptime_secs);
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
                eprintln!("usage: mind <what you want> | mind update | mind doctor | mind status | mind history | mind chat | mind models | mind model <file|auto> | mind download <id> | mind thinking on|off");
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
        }
    }
}
