// Copyright (c) 2026 Black Arrow Software, LLC. All rights reserved.
// Licensed under the MindOS Software License; see LICENSE at the repository root.

//! mindd: the MindOS mind daemon.

use anyhow::{anyhow, Result};
use mindos_mind::config::Config;
use mindos_mind::daemon::{audit::Audit, llm, server, Daemon};
use std::sync::atomic::Ordering;
use std::time::Duration;

#[tokio::main]
async fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let mut config_path = None;
    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "-c" | "--config" => {
                config_path = args.get(i + 1).map(std::path::PathBuf::from);
                i += 1;
            }
            "-h" | "--help" => {
                println!("usage: mindd [-c /etc/mindos/mind.toml]");
                return Ok(());
            }
            "--version" => {
                println!("mindd {}", mindos_mind::VERSION);
                return Ok(());
            }
            a => return Err(anyhow!("unknown argument {}", a)),
        }
        i += 1;
    }
    let config = Config::load(config_path.as_deref())?;
    eprintln!("mindd {} starting", mindos_mind::VERSION);
    std::fs::create_dir_all(&config.daemon.state_dir).ok();

    let base = config.model.external_url.clone().unwrap_or_else(|| format!("http://{}:{}", config.model.host, config.model.port));
    let llm = llm::Llm::new(base.trim_end_matches('/').to_string(), &config.model);
    let audit = Audit::open(config.daemon.log.clone());
    let d = Daemon::new(config, llm, audit);

    // model server supervisor
    let sup = {
        let d = d.clone();
        tokio::spawn(async move { supervise_model(d).await })
    };

    let srv = {
        let d = d.clone();
        tokio::spawn(async move { server::serve(d).await })
    };
    // the update watcher and the health checks
    let watch = {
        let d = d.clone();
        tokio::spawn(async move { mindos_mind::daemon::updates::watch(d).await })
    };
    let health = {
        let d = d.clone();
        tokio::spawn(async move {
            let mins = d.config.updates.health_interval_mins;
            if mins == 0 {
                return;
            }
            tokio::time::sleep(Duration::from_secs(45)).await;
            loop {
                // Scheduled journal/disk/package probes can wait for the game.
                // User-requested and post-update verification remain immediate.
                while mindos_mind::daemon::updates::game_running() {
                    tokio::time::sleep(Duration::from_secs(30)).await;
                }
                let _ = tokio::time::timeout(Duration::from_secs(120), mindos_mind::daemon::health::run_and_notify(&d)).await;
                tokio::time::sleep(Duration::from_secs(mins * 60)).await;
            }
        })
    };

    let mut term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
    tokio::select! {
        _ = tokio::signal::ctrl_c() => eprintln!("mindd: interrupted"),
        _ = term.recv() => eprintln!("mindd: terminating"),
        r = srv => { if let Ok(Err(e)) = r { eprintln!("mindd: server failed: {:#}", e); } }
    }
    sup.abort();
    watch.abort();
    health.abort();
    let _ = std::fs::remove_file(&d.config.daemon.socket);
    Ok(())
}

/// Keep llama-server running (or just watch an external server).
async fn supervise_model(d: std::sync::Arc<Daemon>) {
    if d.config.model.external_url.is_some() {
        loop {
            let ok = d.llm.health().await;
            if ok && !d.is_ready() {
                if let Some(n) = d.llm.model_name().await {
                    *d.model_name.lock().unwrap() = n;
                }
                eprintln!("mindd: external model server is ready ({})", d.model_name());
            }
            d.ready.store(ok, Ordering::Release);
            tokio::time::sleep(Duration::from_secs(5)).await;
        }
    }
    let mut gpu = d.config.model.gpu_layers != 0;
    let mut backoff = 2u64;
    let mut last_model: Option<std::path::PathBuf> = None;
    loop {
        if d.is_sleeping() {
            *d.model_name.lock().unwrap() = "(sleeping)".into();
            d.restart.notified().await;
            continue;
        }
        let model = match llm::select_model(&d.config.model) {
            Ok(m) => m,
            Err(e) => {
                eprintln!("mindd: no model: {:#} (retrying in 30s)", e);
                *d.model_name.lock().unwrap() = "(no model)".into();
                *d.model_path.lock().unwrap() = None;
                tokio::select! {
                    _ = tokio::time::sleep(Duration::from_secs(30)) => {}
                    _ = d.restart.notified() => {}
                }
                continue;
            }
        };
        // default.gguf is a link: show and remember the real file
        let model = std::fs::canonicalize(&model).unwrap_or(model);
        if last_model.as_ref() != Some(&model) {
            // a new model gets a fresh try on the GPU
            gpu = d.config.model.gpu_layers != 0;
            backoff = 2;
            last_model = Some(model.clone());
        }
        let thinking = d.thinking.load(Ordering::Relaxed);
        *d.model_name.lock().unwrap() = model.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        *d.model_path.lock().unwrap() = Some(model.clone());
        eprintln!("mindd: starting llama-server with {} ({}, thinking {})", model.display(), if gpu { "GPU" } else { "CPU" }, if thinking { "on" } else { "off" });
        let mut proc = match llm::spawn_server(&d.config.model, &model, gpu, thinking) {
            Ok(p) => p,
            Err(e) => {
                eprintln!("mindd: {:#}", e);
                tokio::select! {
                    _ = tokio::time::sleep(Duration::from_secs(backoff)) => {}
                    _ = d.restart.notified() => {}
                }
                backoff = (backoff * 2).min(60);
                continue;
            }
        };
        // wait for health (a model change meanwhile starts over)
        let started = std::time::Instant::now();
        let mut healthy = false;
        let mut restart = false;
        while started.elapsed() < Duration::from_secs(300) {
            if let Ok(Some(status)) = proc.child.try_wait() {
                eprintln!("mindd: llama-server exited during startup: {}", status);
                break;
            }
            if d.llm.health().await {
                healthy = true;
                break;
            }
            tokio::select! {
                _ = tokio::time::sleep(Duration::from_millis(500)) => {}
                _ = d.restart.notified() => { restart = true; break; }
            }
        }
        if restart {
            eprintln!("mindd: model change requested; restarting llama-server");
            let _ = proc.child.kill().await;
            continue;
        }
        if !healthy {
            let _ = proc.child.kill().await;
            if gpu {
                eprintln!("mindd: GPU backend failed, falling back to CPU");
                gpu = false;
            } else {
                tokio::select! {
                    _ = tokio::time::sleep(Duration::from_secs(backoff)) => {}
                    _ = d.restart.notified() => {}
                }
                backoff = (backoff * 2).min(60);
            }
            continue;
        }
        backoff = 2;
        eprintln!("mindd: model ready in {:.1}s", started.elapsed().as_secs_f64());
        d.ready.store(true, Ordering::Release);
        // supervise
        loop {
            tokio::select! {
                status = proc.child.wait() => {
                    eprintln!("mindd: llama-server exited: {:?}; restarting", status.ok());
                    break;
                }
                _ = d.restart.notified() => {
                    eprintln!("mindd: model change requested; restarting llama-server");
                    let _ = proc.child.kill().await;
                    break;
                }
                _ = tokio::time::sleep(Duration::from_secs(10)) => {
                    if !d.llm.health().await {
                        // give it a second chance before restarting
                        tokio::time::sleep(Duration::from_secs(5)).await;
                        if !d.llm.health().await {
                            eprintln!("mindd: llama-server unhealthy; restarting");
                            let _ = proc.child.kill().await;
                            break;
                        }
                    }
                }
            }
        }
        d.ready.store(false, Ordering::Release);
    }
}
