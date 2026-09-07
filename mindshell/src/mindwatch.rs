//! A standing connection to the Mind daemon: `subscribe` once, then every
//! notice, update-status and sleep event reaches the main loop as
//! `HostEvent::Mind`. Reconnects while mindd is down.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::time::Duration;

use serde_json::{json, Value};

use crate::mind;
use crate::HostEvent;

pub fn start(events: async_channel::Sender<HostEvent>) {
    std::thread::Builder::new()
        .name("mindshell-mind".into())
        .spawn(move || {
            let mut announced = false;
            loop {
                match connect() {
                    Ok(stream) => {
                        announced = false;
                        let _ = events.send_blocking(HostEvent::Mind(json!({ "type": "connected" })));
                        let mut lines = BufReader::new(stream).lines();
                        while let Some(Ok(line)) = lines.next() {
                            let Ok(v) = serde_json::from_str::<Value>(&line) else { continue };
                            if events.send_blocking(HostEvent::Mind(v)).is_err() {
                                return;
                            }
                        }
                        let _ = events.send_blocking(HostEvent::Mind(json!({ "type": "disconnected" })));
                    }
                    Err(e) => {
                        if !announced {
                            tracing::info!(%e, "Mind daemon not reachable; retrying");
                            announced = true;
                        }
                    }
                }
                if crate::quit_requested() {
                    return;
                }
                std::thread::sleep(Duration::from_secs(5));
            }
        })
        .expect("spawn mind watch thread");
}

fn connect() -> Result<UnixStream, String> {
    let path = mind::socket_path();
    let stream = UnixStream::connect(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    let _ = stream.set_write_timeout(Some(Duration::from_secs(5)));
    let mut w = stream.try_clone().map_err(|e| e.to_string())?;
    w.write_all(b"{\"type\":\"hello\",\"client\":\"mindshell\"}\n{\"type\":\"subscribe\"}\n").map_err(|e| e.to_string())?;
    Ok(stream)
}
