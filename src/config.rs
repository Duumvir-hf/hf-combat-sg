//! `Modloader/config/combat_sg.json`: the mod's only settings.
//!
//! Loading follows the other HighFleet mods: read the file, write the defaults on first run,
//! log a parse error and fall back to the defaults.

use serde::{Deserialize, Serialize};

/// Where the modloader keeps this mod's settings (relative to the game folder).
pub(crate) const PATH: &str = "Modloader/config/combat_sg.json";

#[derive(Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct Config {
    /// Master switch. False = the hook is not installed at all.
    pub(crate) enabled: bool,
    /// Level file loaded for a fleet-vs-fleet battle in which the enemy side is a strike group.
    /// Relative to `Data/Levels/`, as the game names every level.
    pub(crate) level: String,
    /// Log which enemy fleet triggered the switch, the link count and every non-strike battle.
    pub(crate) verbose_logging: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            enabled: true,
            level: "Combat_SG.seria".to_string(),
            verbose_logging: false,
        }
    }
}

pub(crate) fn load() -> Config {
    match std::fs::read_to_string(PATH) {
        Ok(text) => serde_json::from_str(&text).unwrap_or_else(|error| {
            log::error!("hf_combat_sg: bad {PATH}: {error}; using defaults");
            Config::default()
        }),
        Err(_) => {
            let config = Config::default();
            if let Ok(text) = serde_json::to_string_pretty(&config) {
                let _ = std::fs::write(PATH, text);
            }
            config
        }
    }
}
