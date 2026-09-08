//! Session input preferences. Relative game input remains unaccelerated.
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use smithay::input::keyboard::XkbConfig;
use smithay::reexports::input::{AccelProfile, Device, DeviceCapability};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct InputSettings {
    pub keyboard_layout: String,
    pub keyboard_variant: String,
    pub keyboard_options: String,
    pub repeat_rate: i32,
    pub repeat_delay: i32,
    pub mouse_profile: String,
    pub mouse_speed: f64,
    pub mouse_left_handed: bool,
    pub mouse_natural_scroll: bool,
}

impl Default for InputSettings {
    fn default() -> Self {
        Self {
            keyboard_layout: String::new(), keyboard_variant: String::new(),
            keyboard_options: String::new(), repeat_rate: 25, repeat_delay: 200,
            mouse_profile: "default".into(), mouse_speed: 0.0,
            mouse_left_handed: false, mouse_natural_scroll: false,
        }
    }
}

impl InputSettings {
    pub fn xkb(&self) -> XkbConfig<'_> {
        XkbConfig { layout: &self.keyboard_layout, variant: &self.keyboard_variant,
            options: if self.keyboard_options.is_empty() { None } else { Some(self.keyboard_options.clone()) },
            ..XkbConfig::default() }
    }

    pub fn validate(&self) -> Result<(), String> {
        if !(0..=100).contains(&self.repeat_rate) || !(100..=2000).contains(&self.repeat_delay) {
            return Err("Repeat rate must be 0–100 keys/s and delay 100–2000 ms".into());
        }
        if !self.mouse_speed.is_finite() || !(-1.0..=1.0).contains(&self.mouse_speed) {
            return Err("Mouse speed must be between -1 and 1".into());
        }
        if !matches!(self.mouse_profile.as_str(), "default" | "flat" | "adaptive") {
            return Err("Mouse acceleration must be default, flat or adaptive".into());
        }
        for text in [&self.keyboard_layout, &self.keyboard_variant, &self.keyboard_options] {
            if text.len() > 256 || !text.bytes().all(|c| c.is_ascii_alphanumeric() || b"_,-:".contains(&c)) {
                return Err("Keyboard names must use XKB identifiers (letters, digits, _, -, comma or colon)".into());
            }
        }
        Ok(())
    }

    pub fn patched(&self, patch: Value) -> Result<Self, String> {
        let patch = patch.as_object().ok_or("input must be an object")?;
        let mut merged = serde_json::to_value(self).unwrap();
        for (key, value) in patch {
            if merged.get(key).is_none() { return Err(format!("Unknown input setting: {key}")); }
            merged[key] = value.clone();
        }
        let next: Self = serde_json::from_value(merged).map_err(|e| format!("Invalid input settings: {e}"))?;
        next.validate()?;
        Ok(next)
    }
}

pub fn is_mouse(device: &Device) -> bool {
    device.has_capability(DeviceCapability::Pointer) && device.config_tap_finger_count() == 0
}

pub fn apply_mouse(device: &mut Device, settings: &InputSettings) {
    if !is_mouse(device) { return; }
    let profile = match settings.mouse_profile.as_str() {
        "flat" => Some(AccelProfile::Flat),
        "adaptive" => Some(AccelProfile::Adaptive),
        _ => device.config_accel_default_profile(),
    };
    if let Some(profile) = profile {
        if device.config_accel_profiles().contains(&profile) {
            if let Err(e) = device.config_accel_set_profile(profile) {
                tracing::warn!(device = device.name(), ?e, "cannot set mouse acceleration");
            }
        }
    }
    if device.config_accel_is_available() {
        if let Err(e) = device.config_accel_set_speed(settings.mouse_speed) {
            tracing::warn!(device = device.name(), ?e, "cannot set mouse speed");
        }
    }
    if device.config_left_handed_is_available() {
        let _ = device.config_left_handed_set(settings.mouse_left_handed);
    }
    if device.config_scroll_has_natural_scroll() {
        let _ = device.config_scroll_set_natural_scroll_enabled(settings.mouse_natural_scroll);
    }
}

pub fn mouse_info(device: &Device) -> Value {
    json!({
        "name": device.name(),
        "acceleration": device.config_accel_is_available(),
        "profiles": device.config_accel_profiles().iter().map(|p| match p {
            AccelProfile::Flat => "flat", AccelProfile::Adaptive => "adaptive", _ => "other",
        }).collect::<Vec<_>>(),
        "profile": device.config_accel_profile().map(|p| match p {
            AccelProfile::Flat => "flat", AccelProfile::Adaptive => "adaptive", _ => "other",
        }),
        "speed": device.config_accel_speed(),
        "left_handed": device.config_left_handed(),
        "natural_scroll": device.config_scroll_natural_scroll_enabled(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn partial_changes_preserve_other_input_preferences() {
        let old = InputSettings { keyboard_layout: "de".into(), mouse_speed: 0.4, ..Default::default() };
        let next = old.patched(json!({"repeat_rate": 0})).unwrap();
        assert_eq!(next.keyboard_layout, "de");
        assert_eq!(next.mouse_speed, 0.4);
        assert_eq!(next.repeat_rate, 0);
    }
    #[test]
    fn invalid_changes_leave_original_intact() {
        let old = InputSettings::default();
        for patch in [json!({"mouse_speed": 1.01}), json!({"repeat_delay": -1}),
            json!({"repeat_rate": 101}), json!({"mouse_profile": "fast"}),
            json!({"mouse_left_handed": "yes"}), json!({"keyboard_layout": "us\u{0}"}),
            json!({"misspelled": 1}), json!(null)] {
            assert!(old.patched(patch).is_err());
        }
        assert_eq!(old, InputSettings::default());
    }
}
