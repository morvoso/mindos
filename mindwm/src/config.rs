//! mindwm configuration.
//!
//! `/etc/mindos/mindwm.toml` is overlaid by `~/.config/mindos/mindwm.toml`
//! (and `$MINDWM_CONFIG`); later files override individual keys.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};

use serde::Deserialize;
use smithay::backend::renderer::Color32F;

/// The MindOS void (#080a0e): the desktop clear colour. Red is reserved for
/// the kernel and boot stages; the compositor is dark with a cyan accent.
pub const VOID: [f32; 4] = [8.0 / 255.0, 10.0 / 255.0, 14.0 / 255.0, 1.0];
/// Default text colour (#edf2f8).
pub const FOREGROUND: [f32; 4] = [237.0 / 255.0, 242.0 / 255.0, 248.0 / 255.0, 1.0];
/// MindOS green (#3ddc97), shared by the desktop and native chrome.
pub const ACCENT: [f32; 4] = [61.0 / 255.0, 220.0 / 255.0, 151.0 / 255.0, 1.0];

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default)]
pub struct Config {
    pub startup: Startup,
    pub apps: Apps,
    pub mind: Mind,
    pub theme: Theme,
    pub layout: LayoutConfig,
    pub session: Session,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default)]
pub struct Session {
    /// Kiosk mode, used for the login screen (`mindos-greeter`): the
    /// compositor shows only what `[startup].exec` starts. No Mind bar, no
    /// launcher, and no shortcut or IPC request that starts a program or ends
    /// the session (a Quit over IPC is still honoured: that is how the greeter
    /// hands over to the user's session). VT switching stays available.
    pub kiosk: bool,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default)]
pub struct Startup {
    /// Commands spawned (via `sh -c`) once the Wayland socket and XWayland are up.
    pub exec: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct Apps {
    pub terminal: String,
}

impl Default for Apps {
    fn default() -> Self {
        Apps {
            terminal: "kitty".into(),
        }
    }
}

impl Apps {
    /// Keep shell syntax inside the terminal, including pipelines and quotes.
    pub fn terminal_command(&self, command: &str) -> String {
        format!("{} -e sh -c '{}'", self.terminal, command.replace('\'', "'\\''"))
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct Mind {
    pub socket: String,
    /// Let Mind apply changes without asking (the daemon's policy still applies).
    pub autopilot: bool,
    /// Show Mind's tool activity (commands it runs) in the bar. Most people
    /// only want the answer; the Settings app toggles this per user.
    pub show_tools: bool,
}

impl Default for Mind {
    fn default() -> Self {
        Mind {
            socket: "/run/mindos/mind.sock".into(),
            autopilot: false,
            show_tools: false,
        }
    }
}

/// Window layout defaults; the user's choice (Settings, Super+T) is kept in
/// the preferences file and wins over these.
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct LayoutConfig {
    /// `floating` (KDE-like), `dwindle` (Hyprland-like) or `columns` (Niri-like).
    pub mode: String,
    /// Pixels between tiles.
    pub gap: i32,
    /// Pixels between the tiles and the edge of the usable area.
    pub outer_gap: i32,
    /// Floating mode: open every new window maximised (the old "game mode").
    pub open_maximized: bool,
}

impl Default for LayoutConfig {
    fn default() -> Self {
        LayoutConfig {
            mode: "floating".into(),
            gap: 8,
            outer_gap: 8,
            open_maximized: false,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct Theme {
    pub background: String,
    pub foreground: String,
    /// The accent colour (Mind bar lines, selection, wordmark glow).
    pub accent: String,
    /// The colour of a seam: the side of a tile that touches another tile.
    /// Bright and focus-independent, so the two lines either side of a gap
    /// match and two dark windows never read as one.
    pub seam: String,
    /// Draw the "MINDOS" startup screen (wordmark and key hints) until the
    /// shell puts its desktop up.
    pub show_wordmark: bool,
    /// The XCursor theme, and its nominal size in pixels. The defaults are the
    /// MindOS pointer (mindos-cursors): dark glass with a cyan pulse, drawn at
    /// 24, 32, 48 and 64. The user's choice lives in the preferences file.
    pub cursor_theme: String,
    pub cursor_size: u32,
}

impl Default for Theme {
    fn default() -> Self {
        Theme {
            background: "#080a0e".into(),
            foreground: "#edf2f8".into(),
            accent: "#3ddc97".into(),
            seam: "#edf2f8".into(),
            show_wordmark: true,
            cursor_theme: "MindOS".into(),
            cursor_size: 24,
        }
    }
}

impl Config {
    pub fn paths() -> Vec<PathBuf> {
        let mut paths = vec![PathBuf::from("/etc/mindos/mindwm.toml")];
        if let Some(home) = std::env::var_os("HOME") {
            paths.push(PathBuf::from(home).join(".config/mindos/mindwm.toml"));
        }
        if let Some(extra) = std::env::var_os("MINDWM_CONFIG") {
            paths.push(PathBuf::from(extra));
        }
        paths
    }

    pub fn load() -> Config {
        let mut merged = toml::Table::new();
        for path in Self::paths() {
            let Ok(text) = std::fs::read_to_string(&path) else {
                continue;
            };
            match toml::from_str::<toml::Table>(&text) {
                Ok(table) => {
                    merge(&mut merged, table);
                    tracing::info!(path = %path.display(), "loaded config");
                }
                Err(err) => tracing::warn!(path = %path.display(), %err, "ignoring unparsable config"),
            }
        }
        let mut cfg: Config = toml::Value::Table(merged).try_into().unwrap_or_else(|err| {
            tracing::warn!(%err, "invalid config, using defaults");
            Config::default()
        });
        if let Ok(socket) = std::env::var("MIND_SOCKET") {
            cfg.mind.socket = socket;
        }
        cfg
    }

    pub fn background(&self) -> [f32; 4] {
        parse_color(&self.theme.background).unwrap_or(VOID)
    }

    pub fn foreground(&self) -> [f32; 4] {
        parse_color(&self.theme.foreground).unwrap_or(FOREGROUND)
    }

    pub fn accent(&self) -> [f32; 4] {
        parse_color(&self.theme.accent).unwrap_or(ACCENT)
    }
}

/// The accent in force right now, packed as 0xRRGGBB.
///
/// The user picks a palette in Settings > Appearance and it has to reach the
/// window frames, the title bars and the Mind bar. The render paths that draw
/// them are handed a window and a scale, not the configuration, so the colour
/// lives here instead of being threaded through every signature. It changes
/// when the user picks a theme and at no other time; the drawing caches keep
/// the value they were drawn with and redraw themselves when it differs.
static ACCENT_RGB: AtomicU32 = AtomicU32::new(0x3d_dc97);

pub fn accent_rgb() -> u32 {
    ACCENT_RGB.load(Ordering::Relaxed)
}

pub fn accent_color() -> [f32; 4] {
    crate::text::hex(accent_rgb())
}

pub fn set_accent_rgb(rgb: u32) {
    ACCENT_RGB.store(rgb & 0xff_ffff, Ordering::Relaxed);
}

/// The seam colour, kept the same way as the accent and for the same reason:
/// the frame is drawn far from the configuration.
static SEAM_RGB: AtomicU32 = AtomicU32::new(0xed_f2f8);

pub fn seam_rgb() -> u32 {
    SEAM_RGB.load(Ordering::Relaxed)
}

pub fn set_seam_rgb(rgb: u32) {
    SEAM_RGB.store(rgb & 0xff_ffff, Ordering::Relaxed);
}

/// The accent as `#rrggbb`, for the preferences file and the shell.
pub fn accent_hex() -> String {
    format!("#{:06x}", accent_rgb())
}

/// `#rrggbb` (or `#rgb`) as packed 0xRRGGBB; the alpha of an 8-digit colour
/// is dropped, a frame ring draws its own.
pub fn parse_accent(s: &str) -> Option<u32> {
    let c = parse_color(s)?;
    let channel = |v: f32| ((v.clamp(0.0, 1.0) * 255.0).round() as u32) & 0xff;
    Some((channel(c[0]) << 16) | (channel(c[1]) << 8) | channel(c[2]))
}

fn merge(base: &mut toml::Table, overlay: toml::Table) {
    for (key, value) in overlay {
        match (base.get_mut(&key), value) {
            (Some(toml::Value::Table(b)), toml::Value::Table(o)) => merge(b, o),
            (_, value) => {
                base.insert(key, value);
            }
        }
    }
}

/// Parse `#rgb`, `#rrggbb` or `#rrggbbaa`.
pub fn parse_color(s: &str) -> Option<[f32; 4]> {
    let s = s.trim().trim_start_matches('#');
    let hex = |i: usize| u8::from_str_radix(&s[i..i + 2], 16).ok();
    let (r, g, b, a) = match s.len() {
        3 => {
            let h = |i: usize| u8::from_str_radix(&s[i..i + 1], 16).ok().map(|v| v * 17);
            (h(0)?, h(1)?, h(2)?, 255)
        }
        6 => (hex(0)?, hex(2)?, hex(4)?, 255),
        8 => (hex(0)?, hex(2)?, hex(4)?, hex(6)?),
        _ => return None,
    };
    Some([
        r as f32 / 255.0,
        g as f32 / 255.0,
        b as f32 / 255.0,
        a as f32 / 255.0,
    ])
}

pub fn to_color32f(c: [f32; 4]) -> Color32F {
    Color32F::new(c[0], c[1], c[2], c[3])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terminal_keeps_shell_syntax_in_the_child() {
        // `env` stands in for Kitty: execute its child and inspect the result.
        let apps = Apps { terminal: "env".into() };
        let command = apps.terminal_command("printf '%s' \"hello 'world'\" | tr a-z A-Z");
        let output = std::process::Command::new("sh")
            .args(["-c", &command.replacen("env -e", "env", 1)])
            .output().unwrap();
        assert!(output.status.success());
        assert_eq!(output.stdout, b"HELLO 'WORLD'");
    }

    #[test]
    fn terminal_default_and_user_override() {
        assert_eq!(Apps::default().terminal, "kitty");
        let cfg: Config = toml::from_str("[apps]\nterminal = 'ghostty'").unwrap();
        assert_eq!(cfg.apps.terminal_command("htop"), "ghostty -e sh -c 'htop'");
    }

    #[test]
    fn an_accent_survives_the_trip_to_the_frames_and_back() {
        // The shell sends "#rrggbb", the frames cache a u32 and the
        // preferences file is written from that same u32.
        assert_eq!(parse_accent("#67dce5"), Some(0x67dce5));
        assert_eq!(parse_accent("67dce5"), Some(0x67dce5));
        assert_eq!(parse_accent("#fff"), Some(0xffffff));
        // an alpha is not a frame's to keep: the ring sets its own
        assert_eq!(parse_accent("#67dce580"), Some(0x67dce5));
        assert_eq!(parse_accent("not a colour"), None);

        set_accent_rgb(0x67dce5);
        assert_eq!(accent_rgb(), 0x67dce5);
        assert_eq!(accent_hex(), "#67dce5");
        assert_eq!(accent_color(), crate::text::hex(0x67dce5));
        set_accent_rgb(0x3ddc97);
    }
}
