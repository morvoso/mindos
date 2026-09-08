//! JSON stdin keeps gaming settings and passwords out of process arguments.
use serde_json::Value;
use std::io::Write;
use std::process::{Command, Stdio};

pub fn request(params: Value) -> Result<Value, String> {
    let payload = serde_json::to_vec(&params).map_err(|e| e.to_string())?;
    if payload.len() > 65536 { return Err("Gaming request too large".into()); }
    let mut child = Command::new("mindos-play").stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped())
        .spawn().map_err(|_| "Install or update mindos-gaming to use Gaming Center".to_string())?;
    child.stdin.take().ok_or("Gaming helper has no input")?.write_all(&payload).map_err(|e| e.to_string())?;
    let out = child.wait_with_output().map_err(|e| e.to_string())?;
    let value: Value = serde_json::from_slice(&out.stdout).map_err(|_| "Gaming helper returned an invalid response".to_string())?;
    if !out.status.success() { return Err(value.get("error").and_then(Value::as_str).unwrap_or("Gaming operation failed").to_string()); }
    Ok(value)
}
