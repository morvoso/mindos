//! Application index from XDG desktop entries.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::SystemTime;

use serde::Serialize;

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct AppEntry {
    pub id: String,
    pub name: String,
    pub comment: String,
    pub exec: String,
    /// Icon name (or absolute path) from the entry; empty when none.
    pub icon: String,
    /// The icon file the name resolved to in the configured theme, if any.
    pub icon_path: Option<String>,
    pub categories: Vec<String>,
    pub terminal: bool,
    /// `StartupWMClass`: the app_id / WM_CLASS its windows carry when that
    /// differs from the desktop id (Wine's generated entries: `notepad++.exe`).
    pub wm_class: String,
    /// The entry starts a Windows program through Wine (or Proton).
    pub wine: bool,
    /// Lower-cased name, generic name, comment and keywords for matching.
    pub haystack: String,
}

pub fn data_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Some(home) = std::env::var_os("XDG_DATA_HOME") {
        dirs.push(PathBuf::from(home));
    } else if let Some(home) = std::env::var_os("HOME") {
        dirs.push(PathBuf::from(home).join(".local/share"));
    }
    let system = std::env::var("XDG_DATA_DIRS").unwrap_or_else(|_| "/usr/local/share:/usr/share".into());
    dirs.extend(system.split(':').filter(|s| !s.is_empty()).map(PathBuf::from));
    // Flatpak exports are not always in XDG_DATA_DIRS of the session.
    for extra in ["/var/lib/flatpak/exports/share", "/var/lib/snapd/desktop"] {
        let p = PathBuf::from(extra);
        if !dirs.contains(&p) {
            dirs.push(p);
        }
    }
    if let Some(home) = std::env::var_os("HOME") {
        let p = PathBuf::from(home).join(".local/share/flatpak/exports/share");
        if !dirs.contains(&p) {
            dirs.push(p);
        }
    }
    dirs
}

fn application_dirs() -> Vec<PathBuf> {
    data_dirs().into_iter().map(|d| d.join("applications")).collect()
}

/// A fingerprint of the applications directories (mtimes), to know when to rescan.
pub fn fingerprint() -> Vec<(PathBuf, Option<SystemTime>)> {
    let mut out = Vec::new();
    for base in application_dirs() {
        let mut stack = vec![base];
        while let Some(d) = stack.pop() {
            let mtime = std::fs::metadata(&d).and_then(|m| m.modified()).ok();
            out.push((d.clone(), mtime));
            if let Ok(entries) = std::fs::read_dir(&d) {
                for e in entries.flatten() {
                    let p = e.path();
                    if p.is_dir() {
                        stack.push(p);
                    }
                }
            }
        }
    }
    out
}

/// Scan the desktop entries and resolve their icons (call off the main thread).
pub fn load_apps(icon_theme: &str, icon_size: u16) -> Vec<AppEntry> {
    let current_desktop: Vec<String> = std::env::var("XDG_CURRENT_DESKTOP")
        .unwrap_or_else(|_| "MindOS".into())
        .split(':')
        .map(|s| s.to_string())
        .collect();
    let mut apps: BTreeMap<String, AppEntry> = BTreeMap::new();
    for base in application_dirs() {
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
                if let Ok(text) = std::fs::read_to_string(&path) {
                    if let Some(app) = parse_desktop_entry(&text, &id, &current_desktop) {
                        apps.insert(id, app);
                    }
                }
            }
        }
    }
    let mut list: Vec<AppEntry> = apps.into_values().collect();
    for app in &mut list {
        app.icon_path = crate::icons::resolve(&app.icon, icon_size, icon_theme)
            .map(|p| p.to_string_lossy().into_owned());
    }
    list.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    list
}

pub fn parse_desktop_entry(text: &str, id: &str, current_desktop: &[String]) -> Option<AppEntry> {
    let mut in_entry = false;
    let mut name = None;
    let mut exec = None;
    let mut icon = String::new();
    let mut generic = String::new();
    let mut comment = String::new();
    let mut keywords = String::new();
    let mut categories = Vec::new();
    let mut terminal = false;
    let mut wm_class = String::new();
    let mut hidden = false;
    let mut is_app = true;
    let mut only_show_in: Option<Vec<String>> = None;
    let mut not_show_in: Vec<String> = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_entry = line == "[Desktop Entry]";
            continue;
        }
        if !in_entry || line.starts_with('#') {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else { continue };
        let value = value.trim();
        match key.trim() {
            "Type" => is_app = value == "Application",
            "Name" => name = Some(value.to_string()),
            "Exec" => exec = Some(value.to_string()),
            "Icon" => icon = value.to_string(),
            "GenericName" => generic = value.to_string(),
            "Comment" => comment = value.to_string(),
            "Keywords" => keywords = value.replace(';', " "),
            "Categories" => {
                categories = value
                    .split(';')
                    .filter(|s| !s.is_empty())
                    .map(|s| s.to_string())
                    .collect()
            }
            "Terminal" => terminal = value == "true",
            "StartupWMClass" => wm_class = value.to_string(),
            "NoDisplay" | "Hidden" => hidden |= value == "true",
            "OnlyShowIn" => {
                only_show_in = Some(value.split(';').filter(|s| !s.is_empty()).map(|s| s.to_string()).collect())
            }
            "NotShowIn" => not_show_in = value.split(';').filter(|s| !s.is_empty()).map(|s| s.to_string()).collect(),
            _ => {}
        }
    }
    if !is_app || hidden {
        return None;
    }
    if let Some(only) = only_show_in {
        if !only.iter().any(|d| current_desktop.iter().any(|c| c.eq_ignore_ascii_case(d))) {
            return None;
        }
    }
    if not_show_in
        .iter()
        .any(|d| current_desktop.iter().any(|c| c.eq_ignore_ascii_case(d)))
    {
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
        comment: if comment.is_empty() { generic } else { comment },
        exec,
        icon,
        icon_path: None,
        categories,
        terminal,
        wm_class,
        wine,
        haystack,
    })
}

/// Does this (cleaned) Exec line run a Windows program under Wine? Wine's
/// menu builder writes `env WINEPREFIX="..." wine C:\\...`; hand-written
/// entries use `wine`, `wine64` or `wine start`; Proton-backed launchers
/// pass the exe through `proton run`.
pub fn exec_is_wine(exec: &str) -> bool {
    let mut words = exec.split_whitespace().peekable();
    // Skip `env` and its VAR=value assignments.
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

pub fn find<'a>(apps: &'a [AppEntry], id: &str) -> Option<&'a AppEntry> {
    apps.iter()
        .find(|a| a.id == id)
        .or_else(|| apps.iter().find(|a| a.id.eq_ignore_ascii_case(id)))
        .or_else(|| {
            let bare = id.trim_end_matches(".desktop").to_lowercase();
            apps.iter()
                .find(|a| a.id.trim_end_matches(".desktop").to_lowercase() == bare)
        })
}

/// Detach a command line from the shell so it survives a shell restart.
pub fn spawn_detached(cmd: &str, env: &[(String, String)]) -> Result<(), String> {
    tracing::info!(cmd, "launching");
    let mut command = std::process::Command::new("sh");
    command
        .arg("-c")
        .arg(cmd)
        .envs(env.iter().map(|(k, v)| (k.as_str(), v.as_str())))
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    if let Ok(cwd) = std::env::var("HOME") {
        command.current_dir(cwd);
    }
    // Put the child in its own session and process group.
    unsafe {
        use std::os::unix::process::CommandExt;
        command.pre_exec(|| {
            libc::setsid();
            Ok(())
        });
    }
    let mut child = command.spawn().map_err(|e| format!("cannot start `{cmd}`: {e}"))?;
    // Reap in the background so no zombies accumulate.
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_field_codes() {
        assert_eq!(clean_exec("firefox %u"), "firefox");
        assert_eq!(clean_exec("steam %U --flag"), "steam --flag");
        assert_eq!(clean_exec("app 100%% %f"), "app 100%");
    }

    #[test]
    fn recognises_wine_entries() {
        assert!(exec_is_wine("env WINEPREFIX=/home/u/.wine wine C:\\\\Program\\ Files\\\\7-Zip\\\\7zFM.exe"));
        assert!(exec_is_wine("wine64 notepad"));
        assert!(exec_is_wine("/opt/wine-staging/bin/wine start /unix /x.exe"));
        assert!(exec_is_wine("env A=1 B=2 proton run game.exe"));
        assert!(!exec_is_wine("winetricks"));
        assert!(!exec_is_wine("env FOO=bar firefox"));
        assert!(!exec_is_wine("lutris lutris:rungameid/3"));
        assert!(!exec_is_wine(""));

        let text = "[Desktop Entry]\nType=Application\nName=7-Zip File Manager\nExec=env WINEPREFIX=\"/home/u/.wine\" wine C:\\\\\\\\ProgramData\\\\\\\\7-Zip.lnk\nIcon=A0B1_7zFM.0\nStartupWMClass=7zfm.exe\n";
        let app = parse_desktop_entry(text, "wine-Programs-7-Zip-7-Zip File Manager.desktop", &["MindOS".into()]).unwrap();
        assert!(app.wine);
        assert_eq!(app.wm_class, "7zfm.exe");
        assert!(app.haystack.contains("windows"));
    }

    #[test]
    fn parses_entries() {
        let text = "[Desktop Entry]\nType=Application\nName=Steam\nComment=Games\nExec=steam %U\nIcon=steam\nCategories=Game;Network;\nTerminal=false\n\n[Desktop Action Store]\nName=Store\nExec=steam store\n";
        let app = parse_desktop_entry(text, "steam.desktop", &["MindOS".into()]).unwrap();
        assert_eq!(app.name, "Steam");
        assert_eq!(app.exec, "steam");
        assert_eq!(app.icon, "steam");
        assert_eq!(app.categories, vec!["Game", "Network"]);
        assert!(!app.terminal);
        assert!(!app.wine);
        assert_eq!(app.wm_class, "");
        assert!(app.haystack.contains("games"));

        let hidden = "[Desktop Entry]\nType=Application\nName=X\nExec=x\nNoDisplay=true\n";
        assert!(parse_desktop_entry(hidden, "x.desktop", &["MindOS".into()]).is_none());
        let kde_only = "[Desktop Entry]\nType=Application\nName=X\nExec=x\nOnlyShowIn=KDE;\n";
        assert!(parse_desktop_entry(kde_only, "x.desktop", &["MindOS".into()]).is_none());
        let not_here = "[Desktop Entry]\nType=Application\nName=X\nExec=x\nNotShowIn=MindOS;\n";
        assert!(parse_desktop_entry(not_here, "x.desktop", &["MindOS".into()]).is_none());
        let link = "[Desktop Entry]\nType=Link\nName=X\nURL=http://x\n";
        assert!(parse_desktop_entry(link, "x.desktop", &["MindOS".into()]).is_none());
    }
}
