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
    /// Optional manual relay for rooms (`host:port` or `|`-several). Empty
    /// means rooms are hosted inside the app itself (the usual case).
    pub server: String,
    /// Name other people in the room see for this player.
    pub name: String,
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
            server: String::new(),
            name: default_name(),
        }
    }
}

/// `ENIGMATIC_SERVER` overrides the saved sync server address.
pub fn env_server() -> Option<String> {
    std::env::var("ENIGMATIC_SERVER").ok().map(|s| s.trim().to_string()).filter(|s| !s.is_empty())
}

/// `ENIGMATIC_NAME`, then a device or user name from the environment.
pub fn default_name() -> String {
    for var in ["ENIGMATIC_NAME", "COMPUTERNAME", "HOSTNAME", "USERNAME", "USER"] {
        if let Ok(v) = std::env::var(var) {
            let n: String = v.trim().chars().filter(|c| !c.is_control()).take(24).collect();
            if !n.is_empty() {
                return n;
            }
        }
    }
    "listener".into()
}

impl Config {
    fn path() -> Option<PathBuf> {
        dirs::config_dir().map(|d| d.join("enigmatic-player").join("config.json"))
    }

    pub fn load() -> Self {
        let mut cfg: Config = Self::path()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
        // Migrate: `127.0.0.1:8080` was the old hard-coded default, it never
        // was a choice - treat it as "no manual relay".
        if cfg.server == "127.0.0.1:8080" {
            cfg.server = String::new();
        }
        cfg
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
