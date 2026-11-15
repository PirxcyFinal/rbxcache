//! Preferences kept between runs, in `settings.json` in the app's data folder.

use crate::protection::data_dir;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// Closing the window keeps the app running in the notification area.
    pub close_to_tray: bool,
    pub animations: bool,
    /// Follow the cache while Roblox uses it.
    pub watch: bool,
    /// The configs that were loaded when the app last ran.
    pub configs: Vec<SavedConfig>,
    /// Where two configs set the same target and the user picked one: target -> that config's file.
    pub winners: BTreeMap<String, PathBuf>,
    /// The version whose changes were last shown on launch.
    pub seen_version: String,
}

/// A loaded config as it is remembered between runs.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SavedConfig {
    pub path: PathBuf,
    /// Whether it is re-applied to what Roblox writes.
    #[serde(default = "enabled_by_default")]
    pub enabled: bool,
    /// The rules of it that are switched off, by name.
    #[serde(default)]
    pub off: Vec<String>,
}

fn enabled_by_default() -> bool {
    true
}

impl Default for Settings {
    fn default() -> Self {
        Self { close_to_tray: true, animations: true, watch: true, configs: Vec::new(), winners: BTreeMap::new(), seen_version: String::new() }
    }
}

fn path() -> Option<PathBuf> {
    data_dir().ok().map(|dir| dir.join("settings.json"))
}

impl Settings {
    /// The saved settings, or the defaults when there are none or they cannot be read.
    pub fn load() -> Self {
        path().and_then(|p| std::fs::read(p).ok()).and_then(|bytes| serde_json::from_slice(&bytes).ok()).unwrap_or_default()
    }

    /// Saves the settings. A failure only costs the preference, so it is not reported.
    pub fn save(&self) {
        let Some(path) = path() else { return };
        if let (Some(parent), Ok(json)) = (path.parent(), serde_json::to_vec_pretty(self)) {
            let _ = std::fs::create_dir_all(parent);
            let _ = std::fs::write(path, json);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_fields_take_their_defaults() {
        let settings: Settings = serde_json::from_str(r#"{"animations": false}"#).unwrap();
        assert_eq!(settings, Settings { animations: false, ..Settings::default() });
        assert!(Settings::default().close_to_tray);
        let listed: Settings = serde_json::from_str(r#"{"configs": [{"path": "a.json"}]}"#).unwrap();
        assert_eq!(listed.configs, [SavedConfig { path: "a.json".into(), enabled: true, off: Vec::new() }]);
    }
}
