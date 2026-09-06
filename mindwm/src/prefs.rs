//! Preferences the compositor changes at runtime and keeps between sessions
//! (`$XDG_STATE_HOME/mindos/mindwm.json`): the layout mode, whether the Mind
//! bar shows tool activity, and the per-output configuration from the
//! Displays settings. `mindwm.toml` stays the read-only defaults file.

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::layout::LayoutMode;

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
#[serde(default)]
pub struct Prefs {
    pub layout_mode: Option<LayoutMode>,
    pub mind_show_tools: Option<bool>,
    /// The output the shell puts its main panels on.
    pub primary_output: Option<String>,
    pub outputs: BTreeMap<String, OutputPrefs>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
#[serde(default)]
pub struct OutputPrefs {
    pub enabled: Option<bool>,
    /// `WIDTHxHEIGHT@REFRESH_MHZ`, e.g. `2560x1440@143912`.
    pub mode: Option<String>,
    pub scale: Option<f64>,
    /// Logical position of the output's top-left corner.
    pub position: Option<[i32; 2]>,
    /// `normal`, `90`, `180`, `270`, `flipped`, `flipped-90`, ...
    pub transform: Option<String>,
    pub vrr: Option<bool>,
}

impl OutputPrefs {
    pub fn is_empty(&self) -> bool {
        *self == OutputPrefs::default()
    }
}

impl Prefs {
    pub fn path() -> PathBuf {
        if let Some(p) = std::env::var_os("MINDWM_STATE") {
            return PathBuf::from(p);
        }
        let base = std::env::var_os("XDG_STATE_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/state")))
            .unwrap_or_else(|| PathBuf::from("/tmp"));
        base.join("mindos/mindwm.json")
    }

    pub fn load() -> Prefs {
        let path = Self::path();
        match std::fs::read_to_string(&path) {
            Ok(text) => match serde_json::from_str::<Prefs>(&text) {
                Ok(prefs) => {
                    tracing::info!(path = %path.display(), "loaded preferences");
                    prefs
                }
                Err(err) => {
                    tracing::warn!(path = %path.display(), %err, "ignoring unreadable preferences");
                    Prefs::default()
                }
            },
            Err(_) => Prefs::default(),
        }
    }

    /// Write atomically; failures are logged, never fatal.
    pub fn save(&self) {
        let path = Self::path();
        let Some(dir) = path.parent() else {
            return;
        };
        if let Err(err) = std::fs::create_dir_all(dir) {
            tracing::warn!(path = %path.display(), %err, "cannot create the state directory");
            return;
        }
        let tmp = path.with_extension("json.tmp");
        let text = match serde_json::to_string_pretty(self) {
            Ok(t) => t,
            Err(err) => {
                tracing::warn!(%err, "cannot serialise preferences");
                return;
            }
        };
        if let Err(err) = std::fs::write(&tmp, text).and_then(|_| std::fs::rename(&tmp, &path)) {
            tracing::warn!(path = %path.display(), %err, "cannot save preferences");
        }
    }

    pub fn output_mut(&mut self, name: &str) -> &mut OutputPrefs {
        self.outputs.entry(name.to_string()).or_default()
    }

    /// Outputs with a user-chosen position.
    pub fn pinned_positions(&self) -> BTreeMap<String, [i32; 2]> {
        self.outputs
            .iter()
            .filter_map(|(name, o)| o.position.map(|p| (name.clone(), p)))
            .collect()
    }

    /// The JSON the shell sees (`prefs` event / `get_prefs`).
    pub fn to_json(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or_default()
    }
}

/// Encode a video mode the way `OutputPrefs::mode` stores it.
pub fn mode_key(width: i32, height: i32, refresh_mhz: i32) -> String {
    format!("{width}x{height}@{refresh_mhz}")
}

/// Parse a `WIDTHxHEIGHT@REFRESH_MHZ` string.
pub fn parse_mode_key(key: &str) -> Option<(i32, i32, i32)> {
    let (size, refresh) = key.split_once('@')?;
    let (w, h) = size.split_once('x')?;
    Some((w.trim().parse().ok()?, h.trim().parse().ok()?, refresh.trim().parse().ok()?))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_json() {
        let mut prefs = Prefs::default();
        prefs.layout_mode = Some(LayoutMode::Columns);
        prefs.mind_show_tools = Some(true);
        prefs.output_mut("DP-1").mode = Some(mode_key(3840, 2160, 240000));
        prefs.output_mut("DP-1").position = Some([0, 0]);
        let text = serde_json::to_string(&prefs).unwrap();
        let back: Prefs = serde_json::from_str(&text).unwrap();
        assert_eq!(back, prefs);
        assert_eq!(parse_mode_key("3840x2160@240000"), Some((3840, 2160, 240000)));
        assert_eq!(parse_mode_key("nope"), None);
    }

    #[test]
    fn unknown_keys_are_ignored() {
        let back: Prefs = serde_json::from_str(r#"{"layout_mode":"dwindle","future":1}"#).unwrap();
        assert_eq!(back.layout_mode, Some(LayoutMode::Dwindle));
    }
}
