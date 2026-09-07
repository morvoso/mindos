//! The desktop layout (`layout.json`): panels, their widgets and the desktop
//! widgets. The shell rebuilds every window from this data, so edit mode is
//! nothing more than editing it.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::config::config_home;

pub const DEFAULT_LAYOUT_PATH: &str = "/usr/share/mindos/shell/layout.json";
pub const BUILTIN_LAYOUT: &str = include_str!("../data/layout.json");

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[serde(default)]
pub struct Layout {
    pub version: u32,
    pub panels: Vec<Panel>,
    pub desktop: Desktop,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

impl Default for Layout {
    fn default() -> Self {
        Layout {
            version: 1,
            panels: Vec::new(),
            desktop: Desktop::default(),
            extra: Map::new(),
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[serde(default)]
pub struct Panel {
    pub id: String,
    /// `*` = every output, otherwise a connector name.
    pub output: String,
    /// top | bottom | left | right
    pub edge: String,
    /// Thickness in logical pixels (also the exclusive zone).
    pub size: i32,
    /// Percentage of the edge covered (100 = full, 0 = fit the widgets).
    pub length: i32,
    /// start | center | end (when length < 100)
    pub align: String,
    pub margin: i32,
    /// top | bottom (layer-shell layer)
    pub layer: String,
    pub opacity: f64,
    pub autohide: bool,
    pub widgets: Vec<Widget>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

impl Default for Panel {
    fn default() -> Self {
        Panel {
            id: String::new(),
            output: "*".into(),
            edge: "bottom".into(),
            size: 48,
            length: 100,
            align: "center".into(),
            margin: 0,
            layer: "top".into(),
            opacity: 0.92,
            autohide: false,
            widgets: Vec::new(),
            extra: Map::new(),
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Default)]
#[serde(default)]
pub struct Widget {
    pub id: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub config: Value,
    /// Anything else (desktop widgets: output, x, y, w, h).
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Default)]
#[serde(default)]
pub struct Desktop {
    pub wallpaper: Value,
    pub widgets: Vec<Widget>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

impl Layout {
    pub fn user_path() -> PathBuf {
        config_home().join("mindos/shell/layout.json")
    }

    pub fn default_path() -> PathBuf {
        std::env::var_os("MINDSHELL_DEFAULT_LAYOUT")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(DEFAULT_LAYOUT_PATH))
    }

    /// The shipped default: the file in /usr/share (or `$MINDSHELL_DEFAULT_LAYOUT`),
    /// else the copy compiled into the binary.
    pub fn load_default() -> Layout {
        let path = Self::default_path();
        match std::fs::read_to_string(&path) {
            Ok(text) => match serde_json::from_str::<Layout>(&text) {
                Ok(layout) => return layout.sanitized(),
                Err(err) => tracing::warn!(path = %path.display(), %err, "invalid default layout"),
            },
            Err(err) => tracing::debug!(path = %path.display(), %err, "no default layout file, using the built-in one"),
        }
        serde_json::from_str::<Layout>(BUILTIN_LAYOUT)
            .expect("built-in layout is valid")
            .sanitized()
    }

    /// The user's layout if there is a valid one, else the default.
    pub fn load() -> Layout {
        let path = Self::user_path();
        match std::fs::read_to_string(&path) {
            Ok(text) => match serde_json::from_str::<Layout>(&text) {
                Ok(layout) => {
                    tracing::info!(path = %path.display(), "loaded user layout");
                    return layout.sanitized();
                }
                Err(err) => tracing::warn!(path = %path.display(), %err, "invalid user layout, using the default"),
            },
            Err(_) => {}
        }
        Self::load_default()
    }

    /// Parse a layout sent by the UI.
    pub fn from_value(value: Value) -> Result<Layout, String> {
        serde_json::from_value::<Layout>(value)
            .map(|l| l.sanitized())
            .map_err(|e| format!("invalid layout: {e}"))
    }

    /// Clamp values to something the host can build windows from.
    pub fn sanitized(mut self) -> Layout {
        if self.version == 0 {
            self.version = 1;
        }
        let mut seen = std::collections::HashSet::new();
        let mut n = 0;
        for panel in &mut self.panels {
            if panel.id.is_empty() || !seen.insert(panel.id.clone()) {
                n += 1;
                panel.id = format!("panel-{n}");
                seen.insert(panel.id.clone());
            }
            if !matches!(panel.edge.as_str(), "top" | "bottom" | "left" | "right") {
                panel.edge = "bottom".into();
            }
            panel.size = panel.size.clamp(16, 400);
            panel.length = if panel.length <= 0 { 0 } else { panel.length.clamp(10, 100) };
            panel.margin = panel.margin.clamp(0, 200);
            if !matches!(panel.align.as_str(), "start" | "center" | "end") {
                panel.align = "center".into();
            }
            if !matches!(panel.layer.as_str(), "top" | "bottom") {
                panel.layer = "top".into();
            }
            if !(0.0..=1.0).contains(&panel.opacity) || panel.opacity.is_nan() {
                panel.opacity = 0.92;
            }
            if panel.output.is_empty() {
                panel.output = "*".into();
            }
            let mut wseen = std::collections::HashSet::new();
            for (i, w) in panel.widgets.iter_mut().enumerate() {
                if w.id.is_empty() || !wseen.insert(w.id.clone()) {
                    w.id = format!("{}-w{}", panel.id, i);
                    wseen.insert(w.id.clone());
                }
                if w.kind.is_empty() {
                    w.kind = "unknown".into();
                }
                if !w.config.is_object() {
                    w.config = Value::Object(Map::new());
                }
            }
        }
        let mut dseen = std::collections::HashSet::new();
        for (i, w) in self.desktop.widgets.iter_mut().enumerate() {
            if w.id.is_empty() || !dseen.insert(w.id.clone()) {
                w.id = format!("desktop-w{i}");
                dseen.insert(w.id.clone());
            }
            if w.kind.is_empty() {
                w.kind = "unknown".into();
            }
            if !w.config.is_object() {
                w.config = Value::Object(Map::new());
            }
        }
        self
    }

    pub fn to_value(&self) -> Value {
        serde_json::to_value(self).unwrap_or(Value::Null)
    }

    /// Write the user layout atomically.
    pub fn save(&self) -> Result<(), String> {
        let path = Self::user_path();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("cannot create {}: {e}", parent.display()))?;
        }
        let tmp = path.with_extension("json.tmp");
        let text = serde_json::to_string_pretty(self).map_err(|e| e.to_string())?;
        std::fs::write(&tmp, text).map_err(|e| format!("cannot write {}: {e}", tmp.display()))?;
        std::fs::rename(&tmp, &path).map_err(|e| format!("cannot replace {}: {e}", path.display()))?;
        tracing::info!(path = %path.display(), "layout saved");
        Ok(())
    }

    pub fn reset() -> Layout {
        let path = Self::user_path();
        if path.exists() {
            if let Err(err) = std::fs::remove_file(&path) {
                tracing::warn!(path = %path.display(), %err, "cannot remove user layout");
            }
        }
        Self::load_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_layout_parses() {
        let layout: Layout = serde_json::from_str(BUILTIN_LAYOUT).unwrap();
        // One Windows-style bar along the bottom: the task bar centred
        // between two expanding spacers, the tray, Mind and the clock after.
        assert_eq!(layout.panels.len(), 1);
        let bar = &layout.panels[0];
        assert_eq!(bar.edge, "bottom");
        assert_eq!(bar.length, 100, "the bar spans the edge");
        assert_eq!(bar.extra.get("float"), Some(&Value::Bool(false)), "flush with the edge");
        let names: Vec<&str> = bar.widgets.iter().map(|w| w.kind.as_str()).collect();
        assert!(names.contains(&"taskbar"));
        assert!(names.contains(&"mind"));
        assert!(names.contains(&"layout-mode"));
        assert!(!names.contains(&"start"));
        assert_eq!(names.iter().filter(|n| **n == "spacer").count(), 2, "two expanding spacers centre the apps");
        let pos = |k: &str| names.iter().position(|n| *n == k).unwrap();
        assert!(pos("taskbar") < pos("tray") && pos("tray") < pos("mind") && pos("mind") + 1 == pos("clock"), "Mind sits right, just left of the clock");
        assert!(layout.desktop.widgets.is_empty(), "no desktop widgets by default");
        assert_eq!(layout.desktop.extra.get("icons"), Some(&Value::Bool(true)), "desktop icons on");
        let round: Value = layout.to_value();
        assert_eq!(round["panels"][0]["widgets"][0]["type"], layout.panels[0].widgets[0].kind);
    }

    #[test]
    fn sanitizes_bad_values() {
        let v = serde_json::json!({
            "panels": [
                {"edge": "middle", "size": 9000, "widgets": [{"type": "clock"}, {"type": "clock"}]},
                {"id": "", "edge": "top", "length": -5},
                {"id": "x", "edge": "left", "length": 3}
            ],
            "desktop": {"widgets": [{"type": "desktop-clock", "x": 10, "y": 20, "w": 300, "h": 100}]}
        });
        let layout = Layout::from_value(v).unwrap();
        assert_eq!(layout.panels[0].edge, "bottom");
        assert_eq!(layout.panels[0].size, 400);
        assert_ne!(layout.panels[0].widgets[0].id, layout.panels[0].widgets[1].id);
        assert_ne!(layout.panels[0].id, layout.panels[1].id);
        assert_eq!(layout.panels[1].length, 0);
        assert_eq!(layout.panels[2].length, 10);
        assert_eq!(layout.desktop.widgets[0].extra["x"], 10);
        assert!(layout.desktop.widgets[0].config.is_object());
    }
}
