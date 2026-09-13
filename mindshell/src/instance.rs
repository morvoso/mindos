//! One window per system app. Settings, the Game Library, Gaming Center and
//! the Task Manager each listen on a session-local socket while open; a second
//! `mindshell --app NAME` (or `shell.openApp` from any shell process) hands its
//! page to the open one, which turns to it and asks for focus.
use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::time::Duration;
use serde_json::Value;
use crate::HostEvent;

/// The apps that keep to one window.
pub fn single(name: &str) -> bool {
    matches!(name, "settings" | "gaming" | "library" | "tasks")
}

fn path(name: &str) -> Option<PathBuf> {
    let runtime = std::env::var_os("XDG_RUNTIME_DIR")?;
    let display = std::env::var("WAYLAND_DISPLAY").ok()?;
    Some(PathBuf::from(runtime).join(format!("mindshell-app-{name}-{}.sock", display.replace('/', "_"))))
}

/// Hand `{name, page, arg}` to the open window of that app; false if none answers.
pub fn forward(name: &str, value: &Value) -> bool {
    let Some(path) = path(name) else { return false };
    let Ok(mut stream) = UnixStream::connect(path) else { return false };
    let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
    let _ = stream.set_write_timeout(Some(Duration::from_secs(2)));
    if writeln!(stream, "{value}").is_err() { return false; }
    let mut ack = [0; 2];
    stream.read_exact(&mut ack).is_ok() && ack == *b"ok"
}

pub fn start(name: &str, events: async_channel::Sender<HostEvent>) {
    let Some(path) = path(name) else { return };
    if UnixStream::connect(&path).is_ok() { return; }
    let _ = std::fs::remove_file(&path);
    let Ok(listener) = UnixListener::bind(&path) else { return };
    let name = name.to_string();
    std::thread::spawn(move || {
        for mut stream in listener.incoming().flatten() {
            let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
            let mut line = String::new();
            if BufReader::new((&stream).take(16384)).read_line(&mut line).is_err() { continue; }
            let Ok(value) = serde_json::from_str::<Value>(&line) else { continue };
            if value["name"].as_str() != Some(name.as_str()) { continue; }
            if events.send_blocking(HostEvent::AppOpen(value)).is_ok() { let _ = stream.write_all(b"ok"); }
        }
    });
}
