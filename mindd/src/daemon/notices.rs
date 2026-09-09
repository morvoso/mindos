//! Notices: what the Mind wants the user to know. Kept in
//! `<state_dir>/notices.json`, pushed to subscribed clients (the desktop
//! shell shows them as notifications and in the notification centre).

use crate::proto::{Event, Notice, NoticeAction};
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use tokio::sync::mpsc::UnboundedSender;

pub struct Notices {
    path: PathBuf,
    list: Mutex<Vec<Notice>>,
    subscribers: Mutex<Vec<UnboundedSender<Event>>>,
}

pub fn now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

pub fn action(label: &str, kind: &str, arg: Value) -> NoticeAction {
    NoticeAction { label: label.into(), kind: kind.into(), arg }
}

impl Notices {
    pub fn open(state_dir: &Path) -> Notices {
        let path = state_dir.join("notices.json");
        let list: Vec<Notice> = std::fs::read_to_string(&path).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default();
        Notices { path, list: Mutex::new(list), subscribers: Mutex::new(vec![]) }
    }

    fn save(&self, list: &[Notice]) {
        if let Some(p) = self.path.parent() {
            let _ = std::fs::create_dir_all(p);
        }
        let tmp = self.path.with_extension("json.tmp");
        if std::fs::write(&tmp, serde_json::to_string_pretty(list).unwrap_or_default()).is_ok() {
            let _ = std::fs::rename(&tmp, &self.path);
        }
    }

    pub fn broadcast(&self, ev: Event) {
        let mut subs = self.subscribers.lock().unwrap();
        subs.retain(|tx| tx.send(ev.clone()).is_ok());
    }

    pub fn subscribe(&self, tx: UnboundedSender<Event>) {
        self.subscribers.lock().unwrap().push(tx);
    }

    pub fn list(&self) -> Vec<Notice> {
        self.list.lock().unwrap().clone()
    }

    pub fn count(&self) -> usize {
        self.list.lock().unwrap().len()
    }

    pub fn get(&self, id: &str) -> Option<Notice> {
        self.list.lock().unwrap().iter().find(|n| n.id == id).cloned()
    }

    /// Add or replace a notice (same id). Returns true when something changed
    /// for the user (new notice, or a different title/body/level). The file
    /// is rewritten only when the stored notice differs in any field: the
    /// health checks post the same findings every half hour.
    pub fn post(&self, mut n: Notice) -> bool {
        if n.time == 0 {
            n.time = now();
        }
        let changed = {
            let mut list = self.list.lock().unwrap();
            let (changed, dirty) = match list.iter().position(|x| x.id == n.id) {
                Some(i) => {
                    let same = list[i].title == n.title && list[i].body == n.body && list[i].level == n.level;
                    if same {
                        n.time = list[i].time;
                    }
                    let dirty = list[i] != n;
                    if dirty {
                        list[i] = n.clone();
                    }
                    (!same, dirty)
                }
                None => {
                    list.push(n.clone());
                    (true, true)
                }
            };
            if dirty {
                list.sort_by(|a, b| b.time.cmp(&a.time));
                self.save(&list);
            }
            changed
        };
        if changed {
            eprintln!("mindd: notice [{}] {}: {}", n.level, n.id, n.title);
            self.broadcast(Event::Notice(n));
        }
        changed
    }

    /// Remove a notice ("*" = all). Returns the ids removed.
    pub fn dismiss(&self, id: &str) -> Vec<String> {
        let removed: Vec<String> = {
            let mut list = self.list.lock().unwrap();
            let removed: Vec<String> = list.iter().filter(|n| id == "*" || n.id == id).map(|n| n.id.clone()).collect();
            if !removed.is_empty() {
                list.retain(|n| !(id == "*" || n.id == id));
                self.save(&list);
            }
            removed
        };
        for id in &removed {
            self.broadcast(Event::NoticeGone { id: id.clone() });
        }
        removed
    }

    /// Remove every notice whose id starts with `prefix` and is not in `keep`:
    /// one rewrite of the file for the whole batch.
    pub fn retain_prefix(&self, prefix: &str, keep: &[String]) {
        let removed: Vec<String> = {
            let mut list = self.list.lock().unwrap();
            let removed: Vec<String> = list.iter().filter(|n| n.id.starts_with(prefix) && !keep.contains(&n.id)).map(|n| n.id.clone()).collect();
            if !removed.is_empty() {
                list.retain(|n| !removed.contains(&n.id));
                self.save(&list);
            }
            removed
        };
        for id in &removed {
            self.broadcast(Event::NoticeGone { id: id.clone() });
        }
    }

    /// One line per notice for the model's context.
    pub fn summary_text(&self) -> String {
        let list = self.list.lock().unwrap();
        if list.is_empty() {
            return String::from("(none)");
        }
        list.iter().map(|n| format!("- [{}] {} — {}", n.level, n.title, n.body.lines().next().unwrap_or(""))).collect::<Vec<_>>().join("\n")
    }
}
