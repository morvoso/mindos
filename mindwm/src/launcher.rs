//! Application index built from XDG desktop entries, used by the Mind bar
//! launcher and by Mind's `launch_app` client tool.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct AppEntry {
    pub id: String,
    pub name: String,
    pub exec: String,
    pub terminal: bool,
    /// The entry starts a Windows program through Wine (or Proton); the bar
    /// tags it so it is not mistaken for a native one.
    pub wine: bool,
    /// Lower-cased name, generic name, comment and keywords for matching.
    pub haystack: String,
}

fn data_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Some(home) = std::env::var_os("XDG_DATA_HOME") {
        dirs.push(PathBuf::from(home));
    } else if let Some(home) = std::env::var_os("HOME") {
        dirs.push(PathBuf::from(home).join(".local/share"));
    }
    let system = std::env::var("XDG_DATA_DIRS").unwrap_or_else(|_| "/usr/local/share:/usr/share".into());
    dirs.extend(system.split(':').filter(|s| !s.is_empty()).map(PathBuf::from));
    dirs
}

pub fn load_apps() -> Vec<AppEntry> {
    let mut apps: BTreeMap<String, AppEntry> = BTreeMap::new();
    for dir in data_dirs() {
        let base = dir.join("applications");
        let mut stack = vec![base.clone()];
        while let Some(d) = stack.pop() {
            let Ok(entries) = std::fs::read_dir(&d) else { continue };
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    stack.push(path);
                    continue;
                }
                if path.extension().and_then(|e| e.to_str()) != Some("desktop") {
                    continue;
                }
                let id = path
                    .strip_prefix(&base)
                    .unwrap_or(&path)
                    .to_string_lossy()
                    .replace('/', "-");
                // Earlier data dirs take precedence, like the XDG spec says.
                if apps.contains_key(&id) {
                    continue;
                }
                if let Some(app) = parse_desktop_file(&path, &id) {
                    apps.insert(id, app);
                }
            }
        }
    }
    let mut list: Vec<AppEntry> = apps.into_values().collect();
    list.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    list
}

fn parse_desktop_file(path: &Path, id: &str) -> Option<AppEntry> {
    let text = std::fs::read_to_string(path).ok()?;
    let mut in_entry = false;
    let mut name = None;
    let mut exec = None;
    let mut generic = String::new();
    let mut comment = String::new();
    let mut keywords = String::new();
    let mut terminal = false;
    let mut hidden = false;
    let mut is_app = true;
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_entry = line == "[Desktop Entry]";
            continue;
        }
        if !in_entry {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else { continue };
        match key.trim() {
            "Type" => is_app = value.trim() == "Application",
            "Name" => name = Some(value.trim().to_string()),
            "Exec" => exec = Some(value.trim().to_string()),
            "GenericName" => generic = value.trim().to_string(),
            "Comment" => comment = value.trim().to_string(),
            "Keywords" => keywords = value.trim().replace(';', " "),
            "Terminal" => terminal = value.trim() == "true",
            "NoDisplay" | "Hidden" => hidden |= value.trim() == "true",
            _ => {}
        }
    }
    if !is_app || hidden {
        return None;
    }
    let name = name?;
    let exec = clean_exec(&exec?);
    if exec.is_empty() {
        return None;
    }
    let wine = exec_is_wine(&exec);
    let mut haystack = format!("{} {} {} {}", name, generic, comment, keywords).to_lowercase();
    if wine {
        haystack.push_str(" windows wine");
    }
    Some(AppEntry {
        id: id.to_string(),
        name,
        exec,
        terminal,
        wine,
        haystack,
    })
}

/// Does this (cleaned) Exec line run a Windows program under Wine? Wine's
/// menu builder writes `env WINEPREFIX="..." wine C:\\...`; hand-written
/// entries use `wine`, `wine64` or `wine start`; Proton launchers `proton run`.
pub fn exec_is_wine(exec: &str) -> bool {
    let mut words = exec.split_whitespace().peekable();
    if words.peek() == Some(&"env") {
        words.next();
        while words.peek().is_some_and(|w| w.contains('=')) {
            words.next();
        }
    }
    let Some(program) = words.next() else { return false };
    let program = program.rsplit('/').next().unwrap_or(program);
    matches!(program, "wine" | "wine64" | "wine-preloader" | "wine64-preloader" | "proton")
}

/// Strip desktop-entry field codes (%f, %U, ...) so the string can be run by `sh -c`.
pub fn clean_exec(exec: &str) -> String {
    let mut out = String::with_capacity(exec.len());
    let mut chars = exec.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '%' {
            match chars.next() {
                Some('%') => out.push('%'),
                Some(_) | None => {}
            }
        } else {
            out.push(c);
        }
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Rank applications for a query: prefix of the name first, then a word
/// prefix, then any substring of the name, then the description/keywords.
pub fn search(apps: &[AppEntry], query: &str, limit: usize) -> Vec<usize> {
    let q = query.trim().to_lowercase();
    if q.is_empty() {
        return Vec::new();
    }
    let mut scored: Vec<(u8, usize)> = apps
        .iter()
        .enumerate()
        .filter_map(|(i, app)| {
            let name = app.name.to_lowercase();
            let score = if name.starts_with(&q) {
                0
            } else if name.split_whitespace().any(|w| w.starts_with(&q)) {
                1
            } else if name.contains(&q) {
                2
            } else if app.haystack.contains(&q) {
                3
            } else {
                return None;
            };
            Some((score, i))
        })
        .collect();
    scored.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| apps[a.1].name.len().cmp(&apps[b.1].name.len())));
    scored.into_iter().take(limit).map(|(_, i)| i).collect()
}

pub fn find_by_name<'a>(apps: &'a [AppEntry], name: &str) -> Option<&'a AppEntry> {
    let q = name.trim().to_lowercase();
    apps.iter()
        .find(|a| a.name.to_lowercase() == q)
        .or_else(|| apps.iter().find(|a| a.id.to_lowercase().trim_end_matches(".desktop") == q))
        .or_else(|| search(apps, name, 1).first().map(|&i| &apps[i]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognises_wine_exec_lines() {
        assert!(exec_is_wine("env WINEPREFIX=/home/u/.wine wine C:\\\\x.lnk"));
        assert!(exec_is_wine("wine64 notepad"));
        assert!(exec_is_wine("env A=1 proton run game.exe"));
        assert!(!exec_is_wine("winetricks"));
        assert!(!exec_is_wine("env FOO=bar firefox"));
        assert!(!exec_is_wine(""));
    }

    #[test]
    fn strips_field_codes() {
        assert_eq!(clean_exec("firefox %u"), "firefox");
        assert_eq!(clean_exec("steam %U --flag"), "steam --flag");
        assert_eq!(clean_exec("app 100%% %f"), "app 100%");
    }
}
