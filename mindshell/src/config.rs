//! Host configuration: `/etc/mindos/shell.toml` overlaid by
//! `~/.config/mindos/shell.toml`. Every key has a default, so both files are
//! optional.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
#[serde(default)]
pub struct Config {
    pub shell: ShellSection,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default)]
pub struct ShellSection {
    /// XDG icon theme for application, tray and desktop icons. Empty (the
    /// default) follows the desktop's own icon pack — GTK's
    /// `gtk-icon-theme-name`, which the settings portal reports as
    /// `org.gnome.desktop.interface icon-theme`; set it to pin one instead.
    pub icon_theme: String,
    /// WebKit compositing policy: "always" or "never".
    pub hardware_acceleration: String,
    /// Terminal emulator used for `Terminal=true` desktop entries.
    pub terminal: String,
    /// Icon size (logical pixels) requested for application icons.
    pub icon_size: u16,
}

impl Default for ShellSection {
    fn default() -> Self {
        ShellSection {
            icon_theme: String::new(),
            hardware_acceleration: "always".into(),
            terminal: "kitty".into(),
            icon_size: 48,
        }
    }
}

pub fn config_home() -> PathBuf {
    if let Some(dir) = std::env::var_os("XDG_CONFIG_HOME") {
        return PathBuf::from(dir);
    }
    let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("/tmp"));
    home.join(".config")
}

impl Config {
    pub fn paths() -> Vec<PathBuf> {
        let mut paths = vec![PathBuf::from("/etc/mindos/shell.toml")];
        paths.push(config_home().join("mindos/shell.toml"));
        if let Some(extra) = std::env::var_os("MINDSHELL_CONFIG") {
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
        toml::Value::Table(merged).try_into().unwrap_or_else(|err| {
            tracing::warn!(%err, "invalid config, using defaults");
            Config::default()
        })
    }

    pub fn hardware_acceleration(&self) -> bool {
        !matches!(self.shell.hardware_acceleration.trim().to_lowercase().as_str(), "never" | "off" | "false" | "no")
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_and_overlay() {
        let cfg: Config = toml::from_str("[shell]\nicon_theme = \"Papirus\"\n").unwrap();
        assert_eq!(cfg.shell.icon_theme, "Papirus");
        assert_eq!(Config::default().shell.icon_theme, "", "the icon theme follows the desktop by default");
        assert_eq!(cfg.shell.terminal, "kitty");
        assert!(cfg.hardware_acceleration());
        let cfg: Config = toml::from_str("[shell]\nhardware_acceleration = \"never\"\n").unwrap();
        assert!(!cfg.hardware_acceleration());
    }
}
