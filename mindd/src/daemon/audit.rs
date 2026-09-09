//! Append-only JSON-lines audit log of everything the mind did.

use serde_json::{json, Value};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// The log is renamed to `<log>.1` (replacing the previous one) when it
/// reaches this size; `tail` reads across the two.
const ROTATE_AT: u64 = 64 * 1024 * 1024;
/// How far back `tail` reads at once. A History request wants the last few
/// hundred lines of a file that grows for months, not the whole file.
const TAIL_WINDOW: u64 = 1024 * 1024;

pub struct Audit {
    path: PathBuf,
    /// The open log and how many bytes it holds.
    file: Mutex<Option<(std::fs::File, u64)>>,
}

impl Audit {
    pub fn open(path: PathBuf) -> Audit {
        if let Some(p) = path.parent() {
            let _ = std::fs::create_dir_all(p);
        }
        let file = open_append(&path);
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
        let mut file = self.file.lock().unwrap();
        if file.as_ref().map(|(_, size)| size + line.len() as u64 + 1 > ROTATE_AT).unwrap_or(false) {
            *file = None;
            let _ = std::fs::rename(&self.path, rotated(&self.path));
            *file = open_append(&self.path);
        }
        if let Some((f, size)) = file.as_mut() {
            if writeln!(f, "{}", line).is_ok() {
                *size += line.len() as u64 + 1;
            }
        }
    }

    /// The last `n` entries, oldest first: from the log and, when that holds
    /// fewer, from the rotated one before it.
    pub fn tail(&self, n: usize) -> Vec<Value> {
        let mut lines = last_lines(&self.path, n);
        if lines.len() < n {
            let mut older = last_lines(&rotated(&self.path), n - lines.len());
            older.append(&mut lines);
            lines = older;
        }
        lines.iter().filter_map(|l| serde_json::from_str(l).ok()).collect()
    }
}

fn open_append(path: &Path) -> Option<(std::fs::File, u64)> {
    let f = std::fs::OpenOptions::new().create(true).append(true).open(path).ok()?;
    let size = f.metadata().map(|m| m.len()).unwrap_or(0);
    Some((f, size))
}

fn rotated(path: &Path) -> PathBuf {
    let mut s = path.as_os_str().to_owned();
    s.push(".1");
    PathBuf::from(s)
}

/// The last `n` non-empty lines of `path`, oldest first, read from the end:
/// one window, doubled while it holds too few lines (unusually long entries),
/// up to eight windows.
fn last_lines(path: &Path, n: usize) -> Vec<String> {
    if n == 0 {
        return vec![];
    }
    let Ok(mut f) = std::fs::File::open(path) else { return vec![] };
    let len = f.metadata().map(|m| m.len()).unwrap_or(0);
    let mut window = TAIL_WINDOW;
    loop {
        let start = len.saturating_sub(window);
        // one byte before the window says whether it begins on a line
        let mut buf = Vec::new();
        if f.seek(SeekFrom::Start(start.saturating_sub(1))).is_err() || f.read_to_end(&mut buf).is_err() {
            return vec![];
        }
        let mut body: &[u8] = &buf;
        if start > 0 {
            body = match body.iter().position(|&b| b == b'\n') {
                Some(p) => &body[p + 1..],
                None => &[],
            };
        }
        let lines: Vec<&[u8]> = body.split(|&b| b == b'\n').filter(|l| !l.iter().all(u8::is_ascii_whitespace)).collect();
        if lines.len() >= n || start == 0 || window >= TAIL_WINDOW * 8 {
            let skip = lines.len().saturating_sub(n);
            return lines[skip..].iter().map(|l| String::from_utf8_lossy(l).into_owned()).collect();
        }
        window *= 2;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("mindd-audit-{}-{}", std::process::id(), name));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir.join("mind.jsonl")
    }

    fn kinds(entries: &[Value]) -> Vec<String> {
        entries.iter().map(|e| e["kind"].as_str().unwrap().to_string()).collect()
    }

    #[test]
    fn the_tail_is_the_last_lines_in_order() {
        let path = temp("tail");
        let audit = Audit::open(path.clone());
        for i in 0..30 {
            audit.record(&format!("k{i}"), "s", 0, json!({"i": i}));
        }
        // read whole: the same answer as reading the file front to back
        let whole: Vec<Value> = std::fs::read_to_string(&path).unwrap().lines().map(|l| serde_json::from_str(l).unwrap()).collect();
        assert_eq!(audit.tail(5), whole[25..].to_vec());
        assert_eq!(audit.tail(30), whole);
        assert_eq!(audit.tail(1000), whole);
        assert!(audit.tail(0).is_empty());
        assert!(Audit::open(temp("missing").join("none")).tail(5).is_empty());
    }

    #[test]
    fn long_lines_widen_the_window_and_blank_lines_do_not_count() {
        let path = temp("window");
        // 40 entries of about 64 KiB: the last 20 are 1.3 MiB, more than one window
        let audit = Audit::open(path.clone());
        for i in 0..40 {
            audit.record("big", "s", 0, json!({"i": i, "pad": "x".repeat(65_000)}));
        }
        drop(audit);
        std::fs::OpenOptions::new().append(true).open(&path).unwrap().write_all(b"\n   \n").unwrap();
        let audit = Audit::open(path);
        let got = audit.tail(20);
        assert_eq!(got.len(), 20);
        assert_eq!(got[0]["i"], 20);
        assert_eq!(got[19]["i"], 39);
    }

    #[test]
    fn the_log_rotates_and_the_tail_reads_across() {
        let path = temp("rotate");
        let audit = Audit::open(path.clone());
        let pad = "y".repeat(1024 * 1024);
        // each entry is ~1 MiB; the 64th pushes the file past the cap
        for i in 0..70 {
            audit.record(&format!("r{i}"), "s", 0, json!({"pad": pad}));
        }
        let old = rotated(&path);
        assert!(old.exists(), "the full log was renamed to .1");
        let current = std::fs::metadata(&path).unwrap().len();
        assert!(current < 10 * 1024 * 1024, "the new log holds only the entries after the cut ({current} bytes)");
        let got = audit.tail(12);
        assert_eq!(kinds(&got), (58..70).map(|i| format!("r{i}")).collect::<Vec<_>>(), "the tail spans both files");
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }
}
