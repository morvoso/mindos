//! Application index built from XDG desktop entries, used by the Mind bar
//! launcher and by Mind's `launch_app` client tool.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

#[derive(Clone)]
pub struct AppEntry {
    pub id: String,
    pub name: String,
    pub exec: String,
    pub desktop_file: PathBuf,
    pub terminal: bool,
    pub icon: String,
    pub icon_pixels: Option<std::sync::Arc<crate::text::Canvas>>,
    /// The entry starts a Windows program through Wine (or Proton); the bar
    /// tags it so it is not mistaken for a native one.
    pub wine: bool,
    /// Lower-cased name, generic name, comment and keywords for matching.
    pub haystack: String,
}

impl std::fmt::Debug for AppEntry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AppEntry").field("id", &self.id).field("name", &self.name).finish()
    }
}

impl AppEntry {
    pub fn launch_command(&self) -> String {
        if self.terminal || self.desktop_file.as_os_str().is_empty() { return self.exec.clone(); }
        // Let GIO interpret the desktop-entry escaping (Wine paths in
        // particular), field codes and working directory, rather than sh.
        format!("gio launch '{}'", self.desktop_file.to_string_lossy().replace('\'', "'\\''"))
    }
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
    if let Some(home) = std::env::var_os("HOME") {
        dirs.push(PathBuf::from(home).join(".local/share/flatpak/exports/share"));
    }
    dirs.push(PathBuf::from("/var/lib/flatpak/exports/share"));
    dirs
}

pub fn load_apps() -> Vec<AppEntry> {
    let mut apps: BTreeMap<String, AppEntry> = BTreeMap::new();
    let mut seen = std::collections::HashSet::new();
    let mut visited = std::collections::HashSet::new();
    for dir in data_dirs() {
        let base = dir.join("applications");
        let mut stack = vec![base.clone()];
        while let Some(d) = stack.pop() {
            let Ok(real) = d.canonicalize() else { continue };
            if !visited.insert(real) { continue; }
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
                if !seen.insert(id.clone()) {
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
    let mut icon = String::new();
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
            "Icon" => icon = value.trim().to_string(),
            "OnlyShowIn" => hidden |= !value.split(';').any(|v| v == "MindOS"),
            "NotShowIn" => hidden |= value.split(';').any(|v| v == "MindOS"),
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
        desktop_file: path.to_path_buf(),
        terminal,
        icon,
        icon_pixels: None,
        wine,
        haystack,
    })
}

/// Does this (cleaned) Exec line run a Windows program under Wine? Wine's
/// menu builder writes `env WINEPREFIX="..." wine C:\\...`; hand-written
/// entries use `wine`, `wine64` or `wine start`; Proton launchers `proton run`.
pub fn exec_is_wine(exec: &str) -> bool {
    let Ok(argv) = gdk_pixbuf::glib::shell_parse_argv(exec) else { return false };
    let mut words = argv.iter().filter_map(|s| s.to_str()).peekable();
    if words.peek().is_some_and(|w| w.rsplit('/').next() == Some("env")) {
        words.next();
        while words.peek().is_some_and(|w| w.contains('=') || *w == "--") { words.next(); }
    }
    let Some(program) = words.next() else { return false };
    let program = program.rsplit('/').next().unwrap_or(program);
    matches!(program, "wine" | "wine64" | "wine-preloader" | "wine64-preloader" | "proton" | "mindos-win" | "mindos-win-open")
}

/// Small default launch list; available immediately without typing.
pub fn quick_launches(apps: &[AppEntry], limit: usize) -> Vec<usize> {
    ["steam.desktop", "net.lutris.Lutris.desktop", "firefox.desktop", "org.gnome.Nautilus.desktop",
        "octopi.desktop", "discord.desktop", "mindos-settings.desktop"]
        .iter().filter_map(|id| apps.iter().position(|a| a.id == *id)).take(limit).collect()
}

/// Parse entries and decode icons away from the compositor/input thread.
/// Cache decoded pixels and publish only changes, including newly installed Wine apps.
pub fn watch_apps(tx: smithay::reexports::calloop::channel::Sender<Vec<AppEntry>>) {
    std::thread::Builder::new().name("mindwm-apps".into()).spawn(move || {
        let mut previous = Vec::new();
        let mut icons = std::collections::HashMap::new();
        let mut games = Vec::new();
        let mut last_games = std::time::Instant::now() - std::time::Duration::from_secs(60);
        loop {
            let mut apps = load_apps();
            if last_games.elapsed().as_secs() >= 60 {
                games = load_games();
                last_games = std::time::Instant::now();
            }
            apps.extend(games.iter().cloned());
            for app in &mut apps {
                if let Some(cached) = icons.get(&app.icon) {
                    app.icon_pixels = Some(std::sync::Arc::clone(cached));
                } else if let Some(pixels) = load_icon(&app.icon) {
                    icons.insert(app.icon.clone(), pixels.clone());
                    app.icon_pixels = Some(pixels);
                }
            }
            let signature: Vec<_> = apps.iter().map(|a| (a.id.clone(), a.name.clone(), a.exec.clone(), a.icon.clone(), a.haystack.clone(), a.terminal, a.icon_pixels.is_some())).collect();
            if signature != previous {
                previous = signature;
                if tx.send(apps).is_err() { break; }
            }
            std::thread::sleep(std::time::Duration::from_secs(3));
        }
    }).expect("start application index worker");
}

/// One search field reaches installed games and their save/session tools.
fn load_games() -> Vec<AppEntry> {
    let Ok(out) = std::process::Command::new("mindos-games").arg("scan").output() else { return Vec::new() };
    let Ok(value) = serde_json::from_slice::<serde_json::Value>(&out.stdout) else { return Vec::new() };
    let Some(items) = value.get("games").and_then(|v| v.as_array()) else { return Vec::new() };
    let mut result = Vec::new();
    for game in items {
        let (Some(id), Some(name)) = (game["id"].as_str(), game["name"].as_str()) else { continue };
        let arg = format!("'{}'", id.replace('\'', "'\\''"));
        for (action, exec, keywords) in [
            ("Play", format!("mindos-games launch {arg}"), "launch resume play game"),
            ("Session", format!("mindshell --app gaming --page sessions {arg}"), "hold suspend resume session tune"),
            ("Saves", format!("mindshell --app gaming --page saves {arg}"), "saves backup cloud restore"),
            ("Companion", format!("mindshell --app companion {arg}"), "guide wiki video snap companion"),
        ] {
            result.push(AppEntry { id: format!("mindos-game:{id}:{action}"), name: format!("{action} {name}"), exec,
                desktop_file: PathBuf::new(), terminal: false, icon: "input-gaming".into(), icon_pixels: None,
                wine: false, haystack: format!("{name} {keywords}").to_lowercase() });
        }
    }
    result
}

fn load_icon(name: &str) -> Option<std::sync::Arc<crate::text::Canvas>> {
    if name.is_empty() { return None; }
    let path = if Path::new(name).is_absolute() {
        PathBuf::from(name)
    } else {
        let bare = name.strip_suffix(".png").or_else(|| name.strip_suffix(".svg")).unwrap_or(name);
        ["Papirus-Dark", "Papirus", "breeze", "hicolor"].iter().find_map(|theme| {
            freedesktop_icons::lookup(bare).with_size(64).with_theme(theme).with_cache().find()
        })?
    };
    // Ignore abnormally large assets; icons are decorative and optional.
    if path.metadata().ok()?.len() > 4 * 1024 * 1024 { return None; }
    let pixbuf = gdk_pixbuf::Pixbuf::from_file_at_scale(path, 64, 64, true).ok()?;
    let bytes = pixbuf.read_pixel_bytes();
    let pixels = bytes.as_ref();
    let (w, h, stride, channels) = (pixbuf.width(), pixbuf.height(), pixbuf.rowstride(), pixbuf.n_channels());
    let mut canvas = crate::text::Canvas::new(64, 64);
    for y in 0..h {
        for x in 0..w {
            let i = (y * stride + x * channels) as usize;
            let a = if pixbuf.has_alpha() { pixels[i + 3] } else { 255 } as u16;
            let j = (((y + (64 - h) / 2) * 64 + x + (64 - w) / 2) * 4) as usize;
            canvas.data[j..j + 4].copy_from_slice(&[
                (pixels[i + 2] as u16 * a / 255) as u8,
                (pixels[i + 1] as u16 * a / 255) as u8,
                (pixels[i] as u16 * a / 255) as u8, a as u8]);
        }
    }
    Some(std::sync::Arc::new(canvas))
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

/// Rank applications for a query: exact name/desktop ID, then a name prefix, a word
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
            let score = if name == q || app.id.trim_end_matches(".desktop").to_lowercase() == q {
                0
            } else if name.starts_with(&q) {
                1
            } else if name.split_whitespace().any(|w| w.starts_with(&q)) {
                2
            } else if name.contains(&q) {
                3
            } else if app.haystack.contains(&q) {
                4
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
    fn exact_app_id_precedes_its_auxiliary_tools() {
        let entry = |id: &str, name: &str| AppEntry {
            id: id.into(), name: name.into(), exec: "octopi".into(),
            desktop_file: PathBuf::new(), terminal: false, icon: String::new(),
            icon_pixels: None, wine: false, haystack: name.to_lowercase(),
        };
        let apps = vec![entry("octopi-cachecleaner.desktop", "Octopi Cache Cleaner"),
            entry("octopi.desktop", "Software (Octopi)")];
        assert_eq!(search(&apps, "octopi", 2), vec![1, 0]);
    }

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
