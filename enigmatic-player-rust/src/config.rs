use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(default)]
pub struct Config {
    pub theme: String,
    pub dir: String,
    pub vol: f32,
    pub muted: bool,
    pub speed: f32,
    pub reverb: usize,
    pub wet: u32,
    /// Output device name. Empty means the system default.
    pub device: String,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            theme: "tokyonight".into(),
            dir: "~/Music/enigmatic-player".into(),
            vol: 0.7,
            muted: false,
            speed: 1.0,
            reverb: 0,
            wet: 0,
            device: String::new(),
        }
    }
}

impl Config {
    fn path() -> Option<PathBuf> {
        dirs::config_dir().map(|d| d.join("enigmatic-player").join("config.json"))
    }

    pub fn load() -> Self {
        Self::path()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    pub fn save(&self) {
        if let Some(p) = Self::path() {
            if let Some(parent) = p.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            if let Ok(s) = serde_json::to_string_pretty(self) {
                let _ = std::fs::write(p, s);
            }
        }
    }
}

/// Expand a leading `~` to the home directory.
pub fn expand(path: &str) -> PathBuf {
    if let Some(rest) = path.strip_prefix('~') {
        if let Some(home) = dirs::home_dir() {
            return home.join(rest.trim_start_matches(|c: char| c == '/' || c == '\\'));
        }
    }
    PathBuf::from(path)
}
