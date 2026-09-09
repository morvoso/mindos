//! The pointer the compositor draws.
//!
//! Clients that name a shape rather than handing over a surface — through
//! `wp_cursor_shape_v1`, which is what GTK 4 and most toolkits now use — get
//! the shape drawn here, from an XCursor theme, animation frames included.
//! One theme, one size, for every application on the desktop.

use std::{collections::HashMap, io::Read, time::Duration};

use smithay::input::pointer::CursorIcon;
use tracing::warn;
use xcursor::{
    parser::{parse_xcursor, Image},
    CursorTheme,
};

static FALLBACK_CURSOR_DATA: &[u8] = include_bytes!("../resources/cursor.rgba");

/// The pointer theme and size for the whole session.
///
/// GSettings (`org.gnome.desktop.interface cursor-theme` / `cursor-size`) is
/// the desktop-wide source of truth: GTK applications follow it live through
/// the settings portal, and `mindos-session` exports it as `XCURSOR_THEME` /
/// `XCURSOR_SIZE` for everything that only reads the environment. The
/// compositor keeps its own copy in the preferences file so its cursor is
/// right from the first frame, before the shell is up; the Pointer settings
/// write both.
///
/// Called once at startup and again whenever the preference changes, so that
/// programs the compositor starts from then on inherit the new size.
pub fn configure(theme: &str, size: u32) {
    std::env::set_var("XCURSOR_THEME", theme);
    std::env::set_var("XCURSOR_SIZE", size.to_string());
}

/// The theme and size `configure` last set (`main` resolves them from the
/// environment, the preferences and the config, in that order).
pub fn configured() -> (String, u32) {
    (
        std::env::var("XCURSOR_THEME").unwrap_or_else(|_| "MindOS".into()),
        std::env::var("XCURSOR_SIZE")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(24),
    )
}

/// One XCursor theme, with the shapes drawn so far kept loaded.
pub struct Cursor {
    theme: CursorTheme,
    size: u32,
    shapes: HashMap<CursorIcon, Vec<Image>>,
    fallback: Vec<Image>,
}

impl Cursor {
    /// The cursor the environment asks for (see `configure`).
    pub fn load() -> Cursor {
        let (name, size) = configured();
        Cursor::load_theme(&name, size)
    }

    pub fn load_theme(name: &str, size: u32) -> Cursor {
        let mut cursor = Cursor {
            theme: CursorTheme::load(name),
            size,
            shapes: HashMap::new(),
            fallback: vec![Image {
                size: 32,
                width: 64,
                height: 64,
                xhot: 1,
                yhot: 1,
                delay: 1,
                pixels_rgba: Vec::from(FALLBACK_CURSOR_DATA),
                pixels_argb: vec![], //unused
            }],
        };
        // The arrow is what every session starts on: load it now, so a broken
        // theme is one warning at startup rather than one per frame.
        if cursor.images(CursorIcon::Default).is_empty() {
            warn!(theme = name, "no default cursor in the theme; using the built-in arrow");
        }
        cursor
    }

    /// The frame to draw for `icon` at `time`, with its index in the icon's
    /// animation: `(icon, index)` is what a renderer keys its texture cache on,
    /// which is a great deal cheaper than comparing the pixels.
    pub fn get_image(&mut self, icon: CursorIcon, scale: u32, time: Duration) -> (usize, Image) {
        let size = self.size * scale;
        let images = self.images(icon);
        let images = if images.is_empty() { &self.fallback } else { images };
        frame(time.as_millis() as u32, size, images)
    }

    /// Whether `icon` is an animation (a turning "wait", say): the frame
    /// after this one is wanted while it shows.
    pub fn animated(&mut self, icon: CursorIcon, scale: u32) -> bool {
        let size = self.size * scale;
        let images = self.images(icon);
        let images = if images.is_empty() { &self.fallback } else { images };
        let total: u32 = nearest_images(size, images).map(|image| image.delay).sum();
        total > 0 && nearest_images(size, images).nth(1).is_some()
    }

    /// The shapes of one icon, loading it the first time it is asked for.
    /// Empty when the theme has nothing for it (the caller draws the arrow).
    fn images(&mut self, icon: CursorIcon) -> &Vec<Image> {
        self.shapes.entry(icon).or_insert_with(|| {
            // The CSS name first, then the X11 names people's themes still use.
            std::iter::once(icon.name())
                .chain(icon.alt_names().iter().copied())
                .find_map(|name| load_icon(&self.theme, name).ok())
                .unwrap_or_default()
        })
    }
}

fn nearest_images(size: u32, images: &[Image]) -> impl Iterator<Item = &Image> {
    // Follow the nominal size of the cursor to choose the nearest
    let nearest_image = images
        .iter()
        .min_by_key(|image| (size as i32 - image.size as i32).abs())
        .unwrap();

    images
        .iter()
        .filter(move |image| image.width == nearest_image.width && image.height == nearest_image.height)
}

fn frame(mut millis: u32, size: u32, images: &[Image]) -> (usize, Image) {
    let total = nearest_images(size, images).fold(0, |acc, image| acc + image.delay);
    if total == 0 {
        return (0, nearest_images(size, images).next().unwrap().clone());
    }
    millis %= total;

    for (index, img) in nearest_images(size, images).enumerate() {
        if millis < img.delay {
            return (index, img.clone());
        }
        millis -= img.delay;
    }

    unreachable!()
}

#[derive(thiserror::Error, Debug)]
enum Error {
    #[error("the theme has no cursor of that name")]
    NoSuchCursor,
    #[error("Error opening xcursor file: {0}")]
    File(#[from] std::io::Error),
    #[error("Failed to parse XCursor file")]
    Parse,
}

fn load_icon(theme: &CursorTheme, name: &str) -> Result<Vec<Image>, Error> {
    let icon_path = theme.load_icon(name).ok_or(Error::NoSuchCursor)?;
    let mut cursor_file = std::fs::File::open(icon_path)?;
    let mut cursor_data = Vec::new();
    cursor_file.read_to_end(&mut cursor_data)?;
    parse_xcursor(&cursor_data).ok_or(Error::Parse)
}
