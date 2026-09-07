//! mindwm configuration.
//!
//! `/etc/mindos/mindwm.toml` is overlaid by `~/.config/mindos/mindwm.toml`
//! (and `$MINDWM_CONFIG`); later files override individual keys.

use std::path::PathBuf;

use serde::Deserialize;
use smithay::backend::renderer::Color32F;

/// The MindOS void (#05070a): the desktop clear colour. Red is reserved for
/// the kernel and boot stages; the compositor is dark with a cyan accent.
pub const VOID: [f32; 4] = [0.0196, 0.0275, 0.0392, 1.0];
/// Default text colour (#e6edf3).
pub const FOREGROUND: [f32; 4] = [0.902, 0.933, 0.953, 1.0];
/// Electric cyan (#19e3ff), the single accent colour of the MindOS look.
pub const ACCENT: [f32; 4] = [0.098, 0.890, 1.0, 1.0];

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
            terminal: "foot".into(),
        }
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
    /// Draw the "MINDOS" wordmark and key hints when no window is open.
    pub show_wordmark: bool,
}

impl Default for Theme {
    fn default() -> Self {
        Theme {
            background: "#05070a".into(),
            foreground: "#e6edf3".into(),
            accent: "#19e3ff".into(),
            show_wordmark: true,
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
