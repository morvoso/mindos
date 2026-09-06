//! Append-only JSON-lines audit log of everything the mind did.

use serde_json::{json, Value};
use std::io::Write;
use std::path::PathBuf;
use std::sync::Mutex;

pub struct Audit {
    path: PathBuf,
    file: Mutex<Option<std::fs::File>>,
}

impl Audit {
    pub fn open(path: PathBuf) -> Audit {
        if let Some(p) = path.parent() {
            let _ = std::fs::create_dir_all(p);
        }
        let file = std::fs::OpenOptions::new().create(true).append(true).open(&path).ok();
        if file.is_none() {
            eprintln!("mindd: cannot open audit log {} (continuing without it)", path.display());
        }
        Audit { path, file: Mutex::new(file) }
    }

    pub fn record(&self, kind: &str, session: &str, uid: u32, mut data: Value) {
        if let Value::Object(m) = &mut data {
            m.insert("ts".into(), json!(chrono::Local::now().to_rfc3339()));
            m.insert("kind".into(), json!(kind));
            m.insert("session".into(), json!(session));
            m.insert("uid".into(), json!(uid));
        }
        let line = data.to_string();
        if let Some(f) = self.file.lock().unwrap().as_mut() {
            let _ = writeln!(f, "{}", line);
        }
    }

    pub fn tail(&self, n: usize) -> Vec<Value> {
        let Ok(s) = std::fs::read_to_string(&self.path) else { return vec![] };
        let lines: Vec<&str> = s.lines().filter(|l| !l.trim().is_empty()).collect();
        let start = lines.len().saturating_sub(n);
        lines[start..].iter().filter_map(|l| serde_json::from_str(l).ok()).collect()
    }
}
