//! Icon lookup: XDG icon themes through `freedesktop-icons`, plus the
//! `IconThemePath` directories StatusNotifierItems point at.

use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};

use gtk4 as gtk;

/// The icon theme in force, shared with the threads that resolve icons (the
/// desktop entry scanner, the tray host) so a change reaches them too.
#[derive(Clone)]
pub struct ThemeRef(Arc<RwLock<String>>);

impl ThemeRef {
    pub fn new(name: impl Into<String>) -> ThemeRef {
        ThemeRef(Arc::new(RwLock::new(name.into())))
    }

    pub fn get(&self) -> String {
        self.0.read().map(|t| t.clone()).unwrap_or_default()
    }

    /// Store `name`; true when it is not the theme we already had.
    pub fn set(&self, name: String) -> bool {
        let Ok(mut current) = self.0.write() else { return false };
        if *current == name {
            return false;
        }
        *current = name;
        true
    }
}

pub fn mime_for(path: &Path) -> &'static str {
    match path.extension().and_then(|e| e.to_str()).map(|e| e.to_ascii_lowercase()).as_deref() {
        Some("svg") | Some("svgz") => "image/svg+xml",
        Some("png") => "image/png",
        Some("jpg") | Some("jpeg") => "image/jpeg",
        Some("gif") => "image/gif",
        Some("webp") => "image/webp",
        Some("ico") => "image/x-icon",
        Some("xpm") => "image/x-xpixmap",
        _ => "application/octet-stream",
    }
}

/// Resolve an icon name (or absolute path) to a file.
pub fn resolve(name: &str, size: u16, theme: &str) -> Option<PathBuf> {
    let name = name.trim();
    if name.is_empty() {
        return None;
    }
    let path = Path::new(name);
    if path.is_absolute() {
        return path.is_file().then(|| path.to_path_buf());
    }
    // "foo.png" in an entry means the icon "foo".
    let bare = name
        .strip_suffix(".png")
        .or_else(|| name.strip_suffix(".svg"))
        .or_else(|| name.strip_suffix(".xpm"))
        .unwrap_or(name);
    let size = size.max(8);
    let themes: Vec<&str> = if theme.is_empty() { vec!["hicolor"] } else { vec![theme, "hicolor"] };
    for t in themes {
        for s in size_ladder(size) {
            if let Some(p) = freedesktop_icons::lookup(bare)
                .with_size(s)
                .with_theme(t)
                .with_cache()
                .find()
            {
                if p.is_file() && !hidpi_dir(&p) {
                    return Some(p);
                }
            }
        }
    }
    // Last resort: /usr/share/pixmaps and friends.
    for dir in ["/usr/share/pixmaps", "/usr/local/share/pixmaps"] {
        for ext in ["svg", "png", "xpm"] {
            let p = Path::new(dir).join(format!("{bare}.{ext}"));
            if p.is_file() {
                return Some(p);
            }
        }
    }
    None
}

/// The sizes a theme usually ships, for walking out from the size asked for.
const STOCK_SIZES: [u16; 10] = [16, 22, 24, 32, 48, 64, 96, 128, 256, 512];

/// Sizes to look the icon up at, best first: the one asked for, then the larger
/// stock sizes (scaling artwork down beats scaling it up), then the smaller ones.
fn size_ladder(size: u16) -> Vec<u16> {
    let mut sizes = vec![size];
    sizes.extend(STOCK_SIZES.iter().copied().filter(|s| *s > size));
    sizes.extend(STOCK_SIZES.iter().rev().copied().filter(|s| *s < size));
    sizes
}

/// True for a file in a HiDPI directory (`16@3x`, `scalable@2x`). The lookup
/// crate matches those on their nominal size and ignores the scale, so a 48 px
/// request lands in Breeze's `16@3x` - small monochrome artwork blown up - when
/// the theme has no 48 px directory for that context. Skipping them sends the
/// search on to the real 64 px icon.
fn hidpi_dir(path: &Path) -> bool {
    path.components()
        .filter_map(|c| c.as_os_str().to_str())
        .any(|c| c.rsplit_once('@').and_then(|(_, s)| s.strip_suffix('x')).is_some_and(|n| n.parse::<u8>().is_ok()))
}

/// Look for `<name>.{svg,png}` under a custom icon directory (StatusNotifierItem
/// `IconThemePath`), a few levels deep, preferring the size closest to `size`.
pub fn resolve_in_dir(dir: &Path, name: &str, size: u16) -> Option<PathBuf> {
    if name.is_empty() || !dir.is_dir() {
        return None;
    }
    let mut best: Option<(i32, PathBuf)> = None;
    let mut stack = vec![(dir.to_path_buf(), 0u8)];
    while let Some((d, depth)) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&d) else { continue };
        for e in entries.flatten() {
            let p = e.path();
            if p.is_dir() {
                if depth < 5 {
                    stack.push((p, depth + 1));
                }
                continue;
            }
            let stem = p.file_stem().and_then(|s| s.to_str()).unwrap_or("");
            if stem != name {
                continue;
            }
            let ext = p.extension().and_then(|e| e.to_str()).unwrap_or("");
            // score: svg best, then the png whose directory size is closest
            let score = match ext {
                "svg" => 0,
                "png" => {
                    let dir_size = p
                        .components()
                        .filter_map(|c| c.as_os_str().to_str())
                        .filter_map(|c| c.split('x').next().and_then(|n| n.parse::<i32>().ok()))
                        .last()
                        .unwrap_or(size as i32);
                    1 + (dir_size - size as i32).abs()
                }
                _ => continue,
            };
            if best.as_ref().map(|(s, _)| score < *s).unwrap_or(true) {
                best = Some((score, p));
            }
        }
    }
    best.map(|(_, p)| p)
}

/// Directories that may hold icon themes (`<dir>/<theme>/index.theme`).
pub fn theme_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Some(home) = std::env::var_os("HOME") {
        dirs.push(PathBuf::from(home).join(".icons"));
    }
    dirs.extend(crate::apps::data_dirs().into_iter().map(|d| d.join("icons")));
    dirs.push(PathBuf::from("/usr/share/pixmaps"));
    dirs
}

pub fn theme_installed(name: &str) -> bool {
    !name.is_empty() && theme_dirs().iter().any(|d| d.join(name).join("index.theme").is_file())
}

/// The icon pack the desktop is set to: GTK's `gtk-icon-theme-name`, which it
/// takes from the settings portal (`org.gnome.desktop.interface icon-theme`)
/// and from the `gtk-3.0/settings.ini` files, so the shell shows whatever the
/// user picked for their applications. Empty before GTK is initialised.
pub fn desktop_theme() -> String {
    gtk::Settings::default()
        .and_then(|s| s.gtk_icon_theme_name())
        .map(|n| n.to_string())
        .unwrap_or_default()
}

/// The theme to draw icons in: the `shell.icon_theme` override when the config
/// sets one, else the desktop's own icon pack; either way something installed.
pub fn theme_for(configured: &str) -> String {
    let wanted = if configured.trim().is_empty() { desktop_theme() } else { configured.to_string() };
    pick_theme(wanted.trim())
}

/// The best installed theme among the preferred ones (directory names, as
/// `Icon=` lookups use them; `freedesktop_icons::list_themes` reports display names).
pub fn pick_theme(preferred: &str) -> String {
    let candidates = [preferred, "breeze-dark", "breeze", "Papirus-Dark", "Papirus", "Adwaita", "hicolor"];
    for c in candidates {
        if theme_installed(c) {
            return c.to_string();
        }
    }
    "hicolor".into()
}

pub fn percent_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(b as char),
            _ => out.push_str(&format!("%{:02X}", b)),
        }
    }
    out
}

pub fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(v) = u8::from_str_radix(&s[i + 1..i + 3], 16) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        if bytes[i] == b'+' {
            out.push(b' ');
        } else {
            out.push(bytes[i]);
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percent_roundtrip() {
        let s = ":1.42/org/ayatana/NotificationItem/steam";
        assert_eq!(percent_decode(&percent_encode(s)), s);
        assert_eq!(percent_encode("a b"), "a%20b");
        assert_eq!(percent_decode("a%2Fb%"), "a/b%");
    }

    #[test]
    fn sizes_walk_up_before_down() {
        let ladder = size_ladder(48);
        assert_eq!(&ladder[..5], &[48, 64, 96, 128, 256]);
        assert_eq!(ladder.last(), Some(&16));
    }

    #[test]
    fn hidpi_directories_are_skipped() {
        assert!(hidpi_dir(Path::new("/usr/share/icons/breeze-dark/mimetypes/16@3x/text-plain.svg")));
        assert!(hidpi_dir(Path::new("/usr/share/icons/x/scalable@2x/places/folder.svg")));
        assert!(!hidpi_dir(Path::new("/usr/share/icons/breeze-dark/places/64/folder.svg")));
        assert!(!hidpi_dir(Path::new("/usr/share/icons/x/apps/mail@home.png")));
    }

    #[test]
    fn mime() {
        assert_eq!(mime_for(Path::new("x.svg")), "image/svg+xml");
        assert_eq!(mime_for(Path::new("x.PNG")), "image/png");
    }
}
