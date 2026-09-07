//! The Wallpaper portal backend (`org.freedesktop.impl.portal.Wallpaper`).
//! "Set as Background" in Files (Nautilus), Image Viewer (Loupe) and any other
//! application that uses the portal reaches the shell here, and the picture
//! goes into the layout's `desktop.wallpaper`, the file the host already
//! watches, so the desktop follows at once. xdg-desktop-portal is pointed at
//! this backend by `mindos-portals.conf` (`org.freedesktop.impl.portal.Wallpaper=mindos`)
//! and `/usr/share/xdg-desktop-portal/portals/mindos.portal`; the bus name is
//! activatable through `mindos-shell.service`.

use std::collections::HashMap;

use gtk4::gio;
use gtk4::gio::prelude::*;
use serde_json::json;
use zbus::zvariant::{ObjectPath, OwnedValue};

use crate::layout::Layout;

pub const BUS_NAME: &str = "org.freedesktop.impl.portal.desktop.mindos";
const OBJECT_PATH: &str = "/org/freedesktop/portal/desktop";

/// Own the backend name on the session bus from a thread of its own; the
/// shell keeps working without it (a warning) when the bus is not there.
pub fn start() {
    let spawned = std::thread::Builder::new().name("mindshell-portal".into()).spawn(|| {
        let rt = match tokio::runtime::Builder::new_current_thread().enable_all().build() {
            Ok(rt) => rt,
            Err(e) => {
                tracing::warn!(%e, "cannot start the portal runtime");
                return;
            }
        };
        rt.block_on(async {
            match serve().await {
                Ok(_connection) => {
                    tracing::info!(name = BUS_NAME, "wallpaper portal backend up");
                    std::future::pending::<()>().await;
                }
                Err(e) => tracing::warn!(%e, "wallpaper portal backend not available"),
            }
        });
    });
    if let Err(e) = spawned {
        tracing::warn!(%e, "cannot spawn the portal thread");
    }
}

async fn serve() -> zbus::Result<zbus::Connection> {
    zbus::connection::Builder::session()?
        .name(BUS_NAME)?
        .serve_at(OBJECT_PATH, Wallpaper)?
        .build()
        .await
}

struct Wallpaper;

#[zbus::interface(name = "org.freedesktop.impl.portal.Wallpaper")]
impl Wallpaper {
    #[zbus(property)]
    fn version(&self) -> u32 {
        1
    }

    /// Portal response codes: 0 success, 1 cancelled by the user, 2 other.
    /// `options`: `show-preview` (ignored, the desktop itself is the preview)
    /// and `set-on` (`background` | `lockscreen` | `both`).
    #[zbus(name = "SetWallpaperURI")]
    async fn set_wallpaper_uri(
        &self,
        _handle: ObjectPath<'_>,
        app_id: &str,
        _parent_window: &str,
        uri: &str,
        options: HashMap<String, OwnedValue>,
    ) -> u32 {
        let set_on = options
            .get("set-on")
            .and_then(|v| <&str>::try_from(v).ok())
            .unwrap_or("background")
            .to_string();
        tracing::info!(app_id, uri, set_on, "wallpaper portal request");
        if set_on == "lockscreen" {
            // The login screen paints the aurora; nothing to set there yet.
            return 0;
        }
        match local_image(uri) {
            Some(path) => match apply(&path) {
                Ok(()) => 0,
                Err(e) => {
                    tracing::warn!(%e, "cannot set the wallpaper");
                    2
                }
            },
            None => {
                tracing::warn!(uri, "wallpaper portal: not a local image");
                2
            }
        }
    }
}

/// The absolute path of a `file://` URI that is a readable image.
fn local_image(uri: &str) -> Option<String> {
    let path = gio::File::for_uri(uri).path()?;
    (path.is_absolute() && path.is_file() && crate::fs::is_image(&path)).then(|| path.to_string_lossy().to_string())
}

fn apply(path: &str) -> Result<(), String> {
    let mut layout = Layout::load();
    layout.desktop.wallpaper = json!({ "mode": "image", "path": path });
    layout.save()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_local_images() {
        assert!(local_image("https://example.com/a.png").is_none());
        assert!(local_image("file:///definitely/not/here.png").is_none());
        let dir = std::env::temp_dir().join(format!("mindshell-portal-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a b.png"), b"png").unwrap();
        std::fs::write(dir.join("a.txt"), b"txt").unwrap();
        let uri = gio::File::for_path(dir.join("a b.png")).uri();
        assert_eq!(local_image(&uri).as_deref(), Some(dir.join("a b.png").to_str().unwrap()));
        assert!(local_image(&gio::File::for_path(dir.join("a.txt")).uri()).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
