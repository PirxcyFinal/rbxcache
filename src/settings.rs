//! Preferences kept between runs, in `settings.json` in the app's data folder.

use crate::protection::data_dir;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// Closing the window keeps the app running in the notification area.
    pub close_to_tray: bool,
    pub animations: bool,
    /// Follow the cache while Roblox uses it.
    pub watch: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self { close_to_tray: true, animations: true, watch: true }
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
    }
}
