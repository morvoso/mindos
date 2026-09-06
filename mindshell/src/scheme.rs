//! The `mindos://shell/` URI scheme: the UI bundle, icons, tray pixmaps,
//! local image files and their thumbnails.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::rc::Weak;
use std::time::UNIX_EPOCH;

use gtk4::gdk_pixbuf::Pixbuf;
use gtk4::gio;
use gtk4::glib;
use webkit6 as webkit;

use crate::app::{self, App};
use crate::fs;
use crate::icons;

pub const SCHEME: &str = "mindos";
pub const ORIGIN: &str = "mindos://shell";

/// Thumbnails wider than this are not produced (the UI asks for 320 or so).
const MAX_THUMB: i32 = 1024;

type Body = (Vec<u8>, &'static str);

/// What a route produces: bytes right away, or a job for a worker thread
/// (files and thumbnails must not block the main loop).
enum Served {
    Now(Body),
    Later(Box<dyn FnOnce() -> Option<Body> + Send>),
    NotFound,
}

pub fn register(context: &webkit::WebContext, app: Weak<App>) {
    if let Some(sm) = context.security_manager() {
        sm.register_uri_scheme_as_secure(SCHEME);
        sm.register_uri_scheme_as_cors_enabled(SCHEME);
        // "Local" origins may display file: resources: the desktop sets
        // `background-image: url(file:///...)` for image wallpapers.
        sm.register_uri_scheme_as_local(SCHEME);
    }
    context.register_uri_scheme(SCHEME, move |request| {
        let Some(app) = app.upgrade() else {
            fail(request, "shell is shutting down");
            return;
        };
        let uri = request.uri().map(|u| u.to_string()).unwrap_or_default();
        match serve(&app, &uri) {
            Served::Now(body) => finish(request, body),
            Served::Later(job) => {
                let request = request.clone();
                glib::spawn_future_local(async move {
                    match app::blocking(job).await {
                        Some(body) => finish(&request, body),
                        None => fail(&request, "not found"),
                    }
                });
            }
            Served::NotFound => {
                tracing::debug!(uri, "mindos:// resource not found");
                fail(request, "not found");
            }
        }
    });
}

fn finish(request: &webkit::URISchemeRequest, (bytes, mime): Body) {
    let len = bytes.len() as i64;
    let stream = gio::MemoryInputStream::from_bytes(&glib::Bytes::from_owned(bytes));
    request.finish(&stream, len, Some(mime));
}

fn fail(request: &webkit::URISchemeRequest, msg: &str) {
    let mut err = glib::Error::new(gio::IOErrorEnum::NotFound, msg);
    request.finish_error(&mut err);
}

/// Split `mindos://shell/a/b?x=1` into ("/a/b", "x=1").
pub fn split_uri(uri: &str) -> Option<(String, String)> {
    let rest = uri.strip_prefix("mindos://")?;
    let (hostpath, query) = match rest.split_once('?') {
        Some((p, q)) => (p, q.to_string()),
        None => (rest, String::new()),
    };
    let (host, path) = match hostpath.split_once('/') {
        Some((h, p)) => (h, format!("/{p}")),
        None => (hostpath, "/".to_string()),
    };
    if host != "shell" {
        return None;
    }
    Some((path, query))
}

pub fn query_param(query: &str, key: &str) -> Option<String> {
    query
        .split('&')
        .filter_map(|kv| kv.split_once('='))
        .find(|(k, _)| *k == key)
        .map(|(_, v)| icons::percent_decode(v))
}

/// The URL that serves a local image file as is.
pub fn file_url(path: &Path) -> String {
    format!("{ORIGIN}/file/{}", icons::percent_encode(&path.to_string_lossy()))
}

/// The URL of a `width`-wide thumbnail of a local image.
pub fn thumb_url(path: &Path, width: i32) -> String {
    format!("{ORIGIN}/thumb/{}?w={width}", icons::percent_encode(&path.to_string_lossy()))
}

fn serve(app: &App, uri: &str) -> Served {
    let Some((path, query)) = split_uri(uri) else { return Served::NotFound };
    let now = |body: Option<Body>| match body {
        Some(b) => Served::Now(b),
        None => Served::NotFound,
    };
    if let Some(rel) = path.strip_prefix("/app/") {
        return now(serve_app(&app.ui_dir, rel));
    }
    if path == "/app" || path == "/" {
        return now(serve_app(&app.ui_dir, "index.html"));
    }
    if let Some(rest) = path.strip_prefix("/icon/") {
        let name = icons::percent_decode(rest);
        let size = query_param(&query, "size")
            .and_then(|s| s.parse::<u16>().ok())
            .unwrap_or(app.config.shell.icon_size);
        let Some(file) = icons::resolve(&name, size, &app.icon_theme) else { return Served::NotFound };
        let mime = icons::mime_for(&file);
        return now(std::fs::read(&file).ok().map(|b| (b, mime)));
    }
    if let Some(rest) = path.strip_prefix("/tray/") {
        let id = icons::percent_decode(rest);
        return now(app.tray.as_ref().and_then(|t| t.pixmap(&id)).map(|b| (b, "image/png")));
    }
    if let Some(rest) = path.strip_prefix("/file/") {
        let Some(file) = image_path(rest) else { return Served::NotFound };
        let mime = mime_for_ext(&file);
        return Served::Later(Box::new(move || std::fs::read(&file).ok().map(|b| (b, mime))));
    }
    if let Some(rest) = path.strip_prefix("/thumb/") {
        let Some(file) = image_path(rest) else { return Served::NotFound };
        let width = query_param(&query, "w")
            .and_then(|s| s.parse::<i32>().ok())
            .unwrap_or(320)
            .clamp(16, MAX_THUMB);
        return Served::Later(Box::new(move || thumbnail(&file, width)));
    }
    Served::NotFound
}

/// Only absolute paths of image files are served from the file system.
fn image_path(rest: &str) -> Option<PathBuf> {
    let decoded = icons::percent_decode(rest);
    let path = PathBuf::from(decoded);
    (path.is_absolute() && fs::is_image(&path) && path.is_file()).then_some(path)
}

/// A PNG of the image scaled to `width` wide (aspect kept), cached on disk
/// by path, size and modification time.
fn thumbnail(path: &Path, width: i32) -> Option<Body> {
    let meta = std::fs::metadata(path).ok()?;
    let mtime = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let mut hasher = DefaultHasher::new();
    (path, mtime, meta.len(), width).hash(&mut hasher);
    let key = hasher.finish();
    let cache_dir = app::dirs_cache().join("mindos/shell/thumbs");
    let cached = cache_dir.join(format!("{key:016x}.png"));
    if let Ok(bytes) = std::fs::read(&cached) {
        return Some((bytes, "image/png"));
    }
    let pixbuf = Pixbuf::from_file_at_scale(path, width, width * 2, true).ok()?;
    let bytes = pixbuf.save_to_bufferv("png", &[]).ok()?;
    let _ = std::fs::create_dir_all(&cache_dir);
    let tmp = cache_dir.join(format!(".{key:016x}.{}.tmp", std::process::id()));
    if std::fs::write(&tmp, &bytes).is_ok() {
        let _ = std::fs::rename(&tmp, &cached);
    }
    Some((bytes, "image/png"))
}

fn serve_app(ui_dir: &Path, rel: &str) -> Option<Body> {
    let rel = icons::percent_decode(rel);
    let rel = rel.trim_start_matches('/');
    if rel.is_empty() {
        return serve_app(ui_dir, "index.html");
    }
    if let Some(font) = rel.strip_prefix("fonts/") {
        if let Some(found) = find_font(ui_dir, font) {
            return std::fs::read(&found).ok().map(|b| (b, mime_for_ext(&found)));
        }
    }
    let file = safe_join(ui_dir, rel)?;
    let file = if file.is_dir() { file.join("index.html") } else { file };
    std::fs::read(&file).ok().map(|b| (b, mime_for_ext(&file)))
}

fn find_font(ui_dir: &Path, name: &str) -> Option<PathBuf> {
    let name = Path::new(name).file_name()?.to_str()?.to_string();
    let candidates = [
        ui_dir.join("fonts").join(&name),
        PathBuf::from("/usr/share/fonts/mindos").join(&name),
        PathBuf::from("/usr/share/fonts/TTF").join(&name),
        PathBuf::from("/usr/share/fonts/OTF").join(&name),
    ];
    candidates.into_iter().find(|p| p.is_file())
}

/// Join `rel` onto `root` refusing anything that escapes it.
pub fn safe_join(root: &Path, rel: &str) -> Option<PathBuf> {
    let mut out = PathBuf::from(root);
    for part in Path::new(rel).components() {
        match part {
            std::path::Component::Normal(p) => out.push(p),
            std::path::Component::CurDir => {}
            _ => return None,
        }
    }
    let canonical = out.canonicalize().ok()?;
    let root = root.canonicalize().ok()?;
    canonical.starts_with(&root).then_some(canonical)
}

fn mime_for_ext(path: &Path) -> &'static str {
    match path.extension().and_then(|e| e.to_str()).map(|e| e.to_ascii_lowercase()).as_deref() {
        Some("html") | Some("htm") => "text/html; charset=utf-8",
        Some("js") | Some("mjs") => "text/javascript; charset=utf-8",
        Some("css") => "text/css; charset=utf-8",
        Some("json") | Some("map") => "application/json; charset=utf-8",
        Some("svg") => "image/svg+xml",
        Some("png") => "image/png",
        Some("jpg") | Some("jpeg") => "image/jpeg",
        Some("webp") => "image/webp",
        Some("gif") => "image/gif",
        Some("bmp") => "image/bmp",
        Some("avif") => "image/avif",
        Some("jxl") => "image/jxl",
        Some("ico") => "image/x-icon",
        Some("woff2") => "font/woff2",
        Some("woff") => "font/woff",
        Some("ttf") => "font/ttf",
        Some("otf") => "font/otf",
        Some("wasm") => "application/wasm",
        Some("txt") | Some("md") => "text/plain; charset=utf-8",
        _ => "application/octet-stream",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_uris() {
        let (p, q) = split_uri("mindos://shell/app/index.html?kind=panel&id=bottom").unwrap();
        assert_eq!(p, "/app/index.html");
        assert_eq!(query_param(&q, "kind").as_deref(), Some("panel"));
        assert_eq!(query_param(&q, "id").as_deref(), Some("bottom"));
        assert_eq!(query_param(&q, "nope"), None);
        let (p, _) = split_uri("mindos://shell/icon//usr/share/pixmaps/x.png?size=32").unwrap();
        assert_eq!(p, "/icon//usr/share/pixmaps/x.png");
        assert!(split_uri("mindos://other/app").is_none());
        assert!(split_uri("https://shell/app").is_none());
    }

    #[test]
    fn file_routes() {
        let url = thumb_url(Path::new("/usr/share/a b.png"), 320);
        assert_eq!(url, "mindos://shell/thumb/%2Fusr%2Fshare%2Fa%20b.png?w=320");
        let (p, q) = split_uri(&url).unwrap();
        let rest = p.strip_prefix("/thumb/").unwrap();
        assert_eq!(icons::percent_decode(rest), "/usr/share/a b.png");
        assert_eq!(query_param(&q, "w").as_deref(), Some("320"));
        assert!(image_path("relative.png").is_none());
        assert!(image_path("%2Fetc%2Fpasswd").is_none());
        assert_eq!(file_url(Path::new("/x.png")), "mindos://shell/file/%2Fx.png");
    }

    #[test]
    fn safe_join_refuses_escapes() {
        let dir = std::env::temp_dir().join(format!("mindshell-test-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        std::fs::write(dir.join("sub/a.js"), "x").unwrap();
        assert!(safe_join(&dir, "sub/a.js").is_some());
        assert!(safe_join(&dir, "../etc/passwd").is_none());
        assert!(safe_join(&dir, "/etc/passwd").is_none());
        assert!(safe_join(&dir, "sub/../../x").is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
