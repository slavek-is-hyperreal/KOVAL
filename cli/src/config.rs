use serde::{Deserialize, Serialize};
use std::fs::{create_dir_all, File};
use std::io::{Read, Write};
use std::path::PathBuf;

#[derive(Serialize, Deserialize, Default, Clone, Debug)]
pub struct Config {
    pub server_url: Option<String>,
    pub token: Option<String>,
}

impl Config {
    /// Gets the path to the config file at ~/.config/koval/config.json
    pub fn file_path() -> PathBuf {
        let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
        PathBuf::from(home).join(".config").join("koval").join("config.json")
    }

    /// Loads the config from ~/.config/koval/config.json
    pub fn load() -> Self {
        let path = Self::file_path();
        if !path.exists() {
            return Config::default();
        }

        let mut file = match File::open(&path) {
            Ok(f) => f,
            Err(_) => return Config::default(),
        };

        let mut content = String::new();
        if file.read_to_string(&mut content).is_err() {
            return Config::default();
        }

        serde_json::from_str(&content).unwrap_or_default()
    }

    /// Saves the config to ~/.config/koval/config.json
    pub fn save(&self) -> Result<(), String> {
        let path = Self::file_path();
        if let Some(parent) = path.parent() {
            create_dir_all(parent).map_err(|e| format!("Failed to create directory {:?}: {}", parent, e))?;
        }

        let json = serde_json::to_string_pretty(self)
            .map_err(|e| format!("Failed to serialize config: {}", e))?;

        let mut file = File::create(&path)
            .map_err(|e| format!("Failed to create config file {:?}: {}", path, e))?;

        file.write_all(json.as_bytes())
            .map_err(|e| format!("Failed to write config file: {}", e))?;

        Ok(())
    }
}
