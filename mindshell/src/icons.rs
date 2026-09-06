//! Icon lookup: XDG icon themes through `freedesktop-icons`, plus the
//! `IconThemePath` directories StatusNotifierItems point at.

use std::path::{Path, PathBuf};

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
        if let Some(p) = freedesktop_icons::lookup(bare)
            .with_size(size)
            .with_theme(t)
            .with_cache()
            .find()
        {
            if p.is_file() {
                return Some(p);
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
    fn mime() {
        assert_eq!(mime_for(Path::new("x.svg")), "image/svg+xml");
        assert_eq!(mime_for(Path::new("x.PNG")), "image/png");
    }
}
