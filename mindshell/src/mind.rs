//! A small client for the Mind daemon socket: one request, the first event
//! that answers it. The Settings app uses it for the model list, model
//! switching, downloads and the thinking preference.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::time::Duration;

use serde_json::{json, Value};

pub fn socket_path() -> PathBuf {
    std::env::var_os("MIND_SOCKET")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/run/mindos/mind.sock"))
}

/// Requests the daemon answers with nothing at all.
fn fire_and_forget(kind: &str) -> bool {
    matches!(kind, "cancel_download" | "cancel" | "hello")
}

/// Send `request` (a `{"type": ...}` object) and return the first event the
/// daemon sends back other than the `welcome` greeting; `error` events
/// become `Err`.
pub fn request(request: Value) -> Result<Value, String> {
    let kind = request
        .get("type")
        .and_then(Value::as_str)
        .ok_or("mind.request: missing 'type'")?
        .to_string();
    let path = socket_path();
    let stream = UnixStream::connect(&path)
        .map_err(|e| format!("Mind is not running ({}: {e})", path.display()))?;
    let _ = stream.set_read_timeout(Some(Duration::from_secs(20)));
    let _ = stream.set_write_timeout(Some(Duration::from_secs(5)));
    let mut writer = stream.try_clone().map_err(|e| e.to_string())?;
    let hello = json!({ "type": "hello", "client": "mindshell" });
    writer
        .write_all(format!("{hello}\n{request}\n").as_bytes())
        .map_err(|e| format!("mind write: {e}"))?;
    if fire_and_forget(&kind) {
        return Ok(Value::Null);
    }
    for line in BufReader::new(stream).lines() {
        let line = line.map_err(|e| format!("mind read: {e}"))?;
        let Ok(value) = serde_json::from_str::<Value>(&line) else { continue };
        match value.get("type").and_then(Value::as_str) {
            Some("welcome") | None => continue,
            Some("error") => {
                return Err(value
                    .get("message")
                    .and_then(Value::as_str)
                    .unwrap_or("Mind error")
                    .to_string())
            }
            Some(_) => return Ok(value),
        }
    }
    Err("Mind closed the connection".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn socket_and_kinds() {
        assert!(fire_and_forget("cancel_download"));
        assert!(!fire_and_forget("models"));
        std::env::remove_var("MIND_SOCKET");
        assert_eq!(socket_path(), PathBuf::from("/run/mindos/mind.sock"));
        assert!(request(json!({"nope": 1})).is_err());
    }
}
