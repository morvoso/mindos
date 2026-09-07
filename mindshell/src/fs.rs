//! File-system helpers for the Files app and the wallpaper picker: directory
//! listings with mime types and icons, places, the basic operations, and
//! the wallpaper search paths.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use gtk4::gio;
use gtk4::gio::prelude::*;
use gtk4::glib;
use serde_json::{json, Value};

use crate::app::icon_url;
use crate::icons;

pub const IMAGE_EXTENSIONS: &[&str] = &["png", "jpg", "jpeg", "webp", "gif", "bmp", "avif", "jxl", "svg"];

pub fn is_image(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| IMAGE_EXTENSIONS.contains(&e.to_ascii_lowercase().as_str()))
        .unwrap_or(false)
}

pub fn home() -> PathBuf {
    std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("/"))
}

/// Resolve `~` and relative paths against the home directory.
pub fn expand(path: &str) -> PathBuf {
    if path.is_empty() || path == "~" {
        return home();
    }
    if let Some(rest) = path.strip_prefix("~/") {
        return home().join(rest);
    }
    let p = PathBuf::from(path);
    if p.is_absolute() {
        p
    } else {
        home().join(p)
    }
}

/// A file name that stays inside its directory.
pub fn valid_name(name: &str) -> Result<(), String> {
    if name.is_empty() || name == "." || name == ".." || name.contains('/') || name.contains('\0') {
        return Err(format!("invalid name '{name}'"));
    }
    Ok(())
}

fn mime_of(path: &Path, is_dir: bool) -> String {
    if is_dir {
        return "inode/directory".into();
    }
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
    let (mime, _) = gio::content_type_guess(Some(name), None);
    mime.to_string()
}

/// The first icon of the mime type's themed icon that the icon theme has.
fn mime_icon(mime: &str, cache: &mut HashMap<String, String>, theme: &str, size: u16) -> String {
    if let Some(url) = cache.get(mime) {
        return url.clone();
    }
    let mut names: Vec<String> = Vec::new();
    if let Ok(themed) = gio::content_type_get_icon(mime).downcast::<gio::ThemedIcon>() {
        names.extend(themed.names().iter().map(|n| n.to_string()));
    }
    names.push(if mime == "inode/directory" { "folder".into() } else { "text-x-generic".into() });
    let name = names
        .iter()
        .find(|n| icons::resolve(n, size, theme).is_some())
        .cloned()
        .unwrap_or_else(|| names.last().cloned().unwrap_or_default());
    let url = icon_url(&name, size);
    cache.insert(mime.to_string(), url.clone());
    url
}

fn mtime_secs(meta: &std::fs::Metadata) -> f64 {
    meta.modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
}

/// `{"path", "parent", "entries": [{name, path, dir, size, mtime, hidden, symlink, mime, icon, image}]}`.
pub fn list(path: &Path, show_hidden: bool, theme: &str, icon_size: u16) -> Result<Value, String> {
    let dir = std::fs::read_dir(path).map_err(|e| format!("cannot open {}: {e}", path.display()))?;
    let mut cache = HashMap::new();
    let mut entries: Vec<(bool, String, Value)> = Vec::new();
    for entry in dir.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        let hidden = name.starts_with('.');
        if hidden && !show_hidden {
            continue;
        }
        let full = entry.path();
        let symlink = entry.file_type().map(|t| t.is_symlink()).unwrap_or(false);
        // Follow links for the kind and size; a dangling link is shown as a file.
        let meta = std::fs::metadata(&full).or_else(|_| entry.metadata());
        let (is_dir, size, mtime) = match &meta {
            Ok(m) => (m.is_dir(), m.len(), mtime_secs(m)),
            Err(_) => (false, 0, 0.0),
        };
        let mime = mime_of(&full, is_dir);
        let icon = mime_icon(&mime, &mut cache, theme, icon_size);
        let value = json!({
            "name": name,
            "path": full.to_string_lossy(),
            "dir": is_dir,
            "size": size,
            "mtime": mtime,
            "hidden": hidden,
            "symlink": symlink,
            "mime": mime,
            "icon": icon,
            "image": !is_dir && is_image(&full),
        });
        entries.push((is_dir, name.to_lowercase(), value));
    }
    entries.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
    Ok(json!({
        "path": path.to_string_lossy(),
        "parent": path.parent().map(|p| p.to_string_lossy().to_string()),
        "entries": entries.into_iter().map(|e| e.2).collect::<Vec<_>>(),
    }))
}

/// Details for the properties dialog.
pub fn stat(path: &Path) -> Result<Value, String> {
    let meta = std::fs::symlink_metadata(path).map_err(|e| format!("cannot stat {}: {e}", path.display()))?;
    let target = std::fs::metadata(path).ok();
    let is_dir = target.as_ref().map(|m| m.is_dir()).unwrap_or(false);
    let link = if meta.file_type().is_symlink() {
        std::fs::read_link(path).ok().map(|p| p.to_string_lossy().to_string())
    } else {
        None
    };
    let items = if is_dir { std::fs::read_dir(path).map(|d| d.count()).ok() } else { None };
    use std::os::unix::fs::PermissionsExt;
    let mode = target.as_ref().map(|m| m.permissions().mode() & 0o777).unwrap_or(0);
    let perms: String = [(0o400, 'r'), (0o200, 'w'), (0o100, 'x'), (0o40, 'r'), (0o20, 'w'), (0o10, 'x'), (0o4, 'r'), (0o2, 'w'), (0o1, 'x')]
        .iter()
        .map(|(bit, ch)| if mode & bit != 0 { *ch } else { '-' })
        .collect();
    Ok(json!({
        "path": path.to_string_lossy(),
        "name": path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default(),
        "dir": is_dir,
        "size": target.as_ref().map(|m| m.len()).unwrap_or(0),
        "mtime": target.as_ref().map(mtime_secs).unwrap_or(0.0),
        "mime": mime_of(path, is_dir),
        "items": items,
        "link": link,
        "permissions": perms,
        "mode": format!("{mode:o}"),
    }))
}

fn special(kind: glib::UserDirectory) -> Option<PathBuf> {
    glib::user_special_dir(kind).filter(|p| p.is_dir())
}

/// The folder shown as icons on the desktop: the XDG Desktop directory,
/// `~/Desktop` when none is configured; created if missing.
pub fn desktop_dir() -> PathBuf {
    let path = glib::user_special_dir(glib::UserDirectory::Desktop)
        .filter(|p| p != &home())
        .unwrap_or_else(|| home().join("Desktop"));
    if let Err(e) = std::fs::create_dir_all(&path) {
        tracing::warn!(path = %path.display(), %e, "cannot create the Desktop folder");
    }
    path
}

/// Sidebar entries: home, the XDG folders, the root and every mounted drive.
pub fn places() -> Value {
    // `kind` is the UI's vocabulary (ui/src/types.ts Place): home / folder /
    // system / mount.
    let mut list = vec![json!({ "name": "Home", "path": home().to_string_lossy(), "icon": "user-home", "kind": "home" })];
    let folders = [
        (glib::UserDirectory::Desktop, "Desktop", "user-desktop"),
        (glib::UserDirectory::Documents, "Documents", "folder-documents"),
        (glib::UserDirectory::Downloads, "Downloads", "folder-download"),
        (glib::UserDirectory::Pictures, "Pictures", "folder-pictures"),
        (glib::UserDirectory::Music, "Music", "folder-music"),
        (glib::UserDirectory::Videos, "Videos", "folder-videos"),
    ];
    for (kind, name, icon) in folders {
        if let Some(path) = special(kind) {
            if path != home() {
                list.push(json!({ "name": name, "path": path.to_string_lossy(), "icon": icon, "kind": "folder" }));
            }
        }
    }
    list.push(json!({ "name": "System", "path": "/", "icon": "drive-harddisk", "kind": "system" }));
    let monitor = gio::VolumeMonitor::get();
    for mount in monitor.mounts() {
        let Some(path) = mount.root().path() else { continue };
        if path == Path::new("/") {
            continue;
        }
        let icon = mount
            .icon()
            .downcast::<gio::ThemedIcon>()
            .ok()
            .and_then(|t| t.names().first().map(|n| n.to_string()))
            .unwrap_or_else(|| "drive-removable-media".into());
        list.push(json!({
            "name": mount.name().to_string(),
            "path": path.to_string_lossy(),
            "icon": icon,
            "kind": "mount",
            "removable": mount.can_eject() || mount.can_unmount(),
        }));
    }
    Value::Array(list)
}

pub fn mkdir(parent: &Path, name: &str) -> Result<PathBuf, String> {
    valid_name(name)?;
    let path = parent.join(name);
    std::fs::create_dir(&path).map_err(|e| format!("cannot create {}: {e}", path.display()))?;
    Ok(path)
}

pub fn rename(path: &Path, name: &str) -> Result<PathBuf, String> {
    valid_name(name)?;
    let parent = path.parent().ok_or("cannot rename the root")?;
    let target = parent.join(name);
    if target.exists() && target != path {
        return Err(format!("'{name}' already exists"));
    }
    std::fs::rename(path, &target).map_err(|e| format!("cannot rename {}: {e}", path.display()))?;
    Ok(target)
}

/// Move to the freedesktop trash (GIO handles the per-volume trash folders).
pub fn trash(paths: &[PathBuf]) -> Result<usize, String> {
    let mut n = 0;
    for path in paths {
        gio::File::for_path(path)
            .trash(gio::Cancellable::NONE)
            .map_err(|e| format!("cannot trash {}: {e}", path.display()))?;
        n += 1;
    }
    Ok(n)
}

/// Copy or move into `dest` without overwriting; `cp -a`/`mv -n` handle
/// directories, permissions and cross-device moves.
pub fn transfer(paths: &[PathBuf], dest: &Path, moving: bool) -> Result<usize, String> {
    if !dest.is_dir() {
        return Err(format!("{} is not a folder", dest.display()));
    }
    let mut n = 0;
    for path in paths {
        let Some(name) = path.file_name() else { continue };
        let target = dest.join(name);
        if target.exists() {
            return Err(format!("'{}' already exists in {}", name.to_string_lossy(), dest.display()));
        }
        if moving && dest.starts_with(path) {
            return Err(format!("cannot move {} into itself", path.display()));
        }
        let mut cmd = std::process::Command::new(if moving { "mv" } else { "cp" });
        if !moving {
            cmd.arg("-a");
        }
        cmd.arg("-n").arg("--").arg(path).arg(&target);
        let out = cmd.output().map_err(|e| format!("cannot run {}: {e}", if moving { "mv" } else { "cp" }))?;
        if !out.status.success() {
            return Err(String::from_utf8_lossy(&out.stderr).trim().to_string());
        }
        n += 1;
    }
    Ok(n)
}

/// The command line that opens `path` with its default application, plus
/// whether that application wants a terminal.
pub fn opener(path: &Path) -> Option<(String, bool)> {
    let is_dir = path.is_dir();
    let mime = mime_of(path, is_dir);
    let info = gio::AppInfo::default_for_type(&mime, false).or_else(|| {
        // No default for the exact type: text-ish files open in the text editor.
        if !is_dir && mime.starts_with("text/") || mime == "application/json" {
            gio::AppInfo::default_for_type("text/plain", false)
        } else {
            None
        }
    })?;
    let cmdline = info.commandline()?.to_string_lossy().to_string();
    // Terminal=true entries: gio has no accessor for it here, read the desktop file id.
    let terminal = info
        .id()
        .map(|id| desktop_entry_wants_terminal(&id))
        .unwrap_or(false);
    let exec = crate::apps::clean_exec(&cmdline);
    Some((format!("{exec} {}", crate::app::shell_quote(&path.to_string_lossy())), terminal))
}

/// Whether the desktop entry `id` (e.g. `foot.desktop`) has `Terminal=true`.
fn desktop_entry_wants_terminal(id: &str) -> bool {
    for dir in crate::apps::data_dirs() {
        let path = dir.join("applications").join(id);
        if let Ok(text) = std::fs::read_to_string(&path) {
            return text.lines().any(|l| l.trim().eq_ignore_ascii_case("terminal=true"));
        }
    }
    false
}

/// Images in the usual wallpaper folders: `[{path, name, folder}]`.
pub fn wallpapers() -> Value {
    let home = home();
    let mut dirs: Vec<PathBuf> = vec![
        PathBuf::from("/usr/share/mindos/wallpapers"),
        PathBuf::from("/usr/share/backgrounds"),
        home.join(".local/share/mindos/wallpapers"),
        home.join("Pictures/Wallpapers"),
        home.join("Pictures"),
    ];
    if let Some(p) = special(glib::UserDirectory::Pictures) {
        if !dirs.contains(&p) {
            dirs.push(p.join("Wallpapers"));
            dirs.push(p);
        }
    }
    let mut seen = std::collections::HashSet::new();
    let mut out: Vec<(String, Value)> = Vec::new();
    let mut push = |path: PathBuf, folder: &Path, out: &mut Vec<(String, Value)>| {
        if !is_image(&path) || !seen.insert(path.clone()) {
            return;
        }
        let name = path.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
        let entry = json!({ "path": path.to_string_lossy(), "name": name, "folder": folder.to_string_lossy() });
        out.push((name.to_lowercase(), entry));
    };
    let theme = PathBuf::from("/usr/share/mindos/wallpaper.png");
    if theme.is_file() {
        push(theme, Path::new("/usr/share/mindos"), &mut out);
    }
    for dir in &dirs {
        let Ok(rd) = std::fs::read_dir(dir) else { continue };
        for entry in rd.flatten() {
            let path = entry.path();
            if path.is_dir() {
                // /usr/share/backgrounds/<vendor>/*.png, one level down.
                if let Ok(sub) = std::fs::read_dir(&path) {
                    for e in sub.flatten() {
                        let p = e.path();
                        if p.is_file() {
                            push(p, &path, &mut out);
                        }
                    }
                }
            } else if path.is_file() {
                push(path, dir, &mut out);
            }
        }
    }
    // KDE wallpaper packages: /usr/share/wallpapers/<Name>/contents/images/<WxH>.png (largest wins).
    if let Ok(rd) = std::fs::read_dir("/usr/share/wallpapers") {
        for entry in rd.flatten() {
            let images = entry.path().join("contents/images");
            let Ok(files) = std::fs::read_dir(&images) else { continue };
            let best = files
                .flatten()
                .map(|e| e.path())
                .filter(|p| p.is_file() && is_image(p))
                .max_by_key(|p| std::fs::metadata(p).map(|m| m.len()).unwrap_or(0));
            if let Some(p) = best {
                let folder = entry.path();
                if seen.insert(p.clone()) {
                    let name = entry.file_name().to_string_lossy().to_string();
                    out.push((
                        name.to_lowercase(),
                        json!({ "path": p.to_string_lossy(), "name": name, "folder": folder.to_string_lossy() }),
                    ));
                }
            }
        }
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    Value::Array(out.into_iter().map(|e| e.1).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_and_paths() {
        assert!(valid_name("a.txt").is_ok());
        assert!(valid_name("..").is_err());
        assert!(valid_name("a/b").is_err());
        assert!(is_image(Path::new("/x/y.PNG")));
        assert!(!is_image(Path::new("/x/y.txt")));
        std::env::set_var("HOME", "/home/test");
        assert_eq!(expand("~/Pictures"), PathBuf::from("/home/test/Pictures"));
        assert_eq!(expand("/usr"), PathBuf::from("/usr"));
        assert_eq!(expand(""), PathBuf::from("/home/test"));
    }

    #[test]
    fn lists_and_edits_a_folder() {
        let dir = std::env::temp_dir().join(format!("mindshell-fs-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        std::fs::write(dir.join("b.txt"), "hello").unwrap();
        std::fs::write(dir.join(".hidden"), "x").unwrap();
        let listing = list(&dir, false, "hicolor", 32).unwrap();
        let names: Vec<&str> = listing["entries"].as_array().unwrap().iter().map(|e| e["name"].as_str().unwrap()).collect();
        assert_eq!(names, vec!["sub", "b.txt"]);
        assert_eq!(listing["entries"][1]["mime"], "text/plain");
        assert_eq!(listing["entries"][0]["dir"], true);
        let all = list(&dir, true, "hicolor", 32).unwrap();
        assert_eq!(all["entries"].as_array().unwrap().len(), 3);
        let made = mkdir(&dir, "new").unwrap();
        assert!(made.is_dir());
        let renamed = rename(&made, "newer").unwrap();
        assert!(renamed.is_dir() && !made.exists());
        assert!(rename(&renamed, "sub").is_err());
        assert_eq!(transfer(&[dir.join("b.txt")], &renamed, false).unwrap(), 1);
        assert!(renamed.join("b.txt").is_file() && dir.join("b.txt").is_file());
        assert!(transfer(&[dir.join("b.txt")], &renamed, false).is_err());
        assert_eq!(transfer(&[dir.join("b.txt")], &dir.join("sub"), true).unwrap(), 1);
        assert!(!dir.join("b.txt").exists() && dir.join("sub/b.txt").is_file());
        let st = stat(&dir.join("sub")).unwrap();
        assert_eq!(st["items"], 1);
        assert_eq!(st["dir"], true);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
