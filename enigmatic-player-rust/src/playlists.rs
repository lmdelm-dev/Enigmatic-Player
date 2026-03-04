use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::library::Track;

#[derive(Serialize, Deserialize, Clone)]
struct SavedTrack {
    title: String,
    artist: String,
    secs: f64,
    path: Option<String>,
    url: Option<String>,
}

#[derive(Serialize, Deserialize, Clone)]
struct SavedList {
    name: String,
    tracks: Vec<SavedTrack>,
}

#[derive(Clone)]
pub struct Playlist {
    pub name: String,
    pub tracks: Vec<Track>,
}

impl Playlist {
    pub fn total_secs(&self) -> f64 {
        self.tracks.iter().map(|t| t.secs).sum()
    }

    /// True when the same file or the same online url is already in the list.
    pub fn contains(&self, t: &Track) -> bool {
        self.tracks
            .iter()
            .any(|x| (x.path.is_some() && x.path == t.path) || (x.url.is_some() && x.url == t.url))
    }
}

fn file() -> Option<PathBuf> {
    dirs::config_dir().map(|d| d.join("enigmatic-player").join("playlists.json"))
}

pub fn load() -> Vec<Playlist> {
    let saved: Vec<SavedList> = file()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default();

    saved
        .into_iter()
        .map(|l| Playlist {
            name: l.name,
            tracks: l
                .tracks
                .into_iter()
                .map(|t| Track::new(t.title, t.artist, t.secs, t.path.map(PathBuf::from), t.url))
                .collect(),
        })
        .collect()
}

/// Saves through a temporary file so a crash can never leave a half-written list.
pub fn save(lists: &[Playlist]) {
    let Some(p) = file() else { return };
    if let Some(parent) = p.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let saved: Vec<SavedList> = lists
        .iter()
        .map(|l| SavedList {
            name: l.name.clone(),
            tracks: l
                .tracks
                .iter()
                .map(|t| SavedTrack {
                    title: t.title.clone(),
                    artist: t.artist.clone(),
                    secs: t.secs,
                    path: t.path.as_ref().map(|x| x.to_string_lossy().to_string()),
                    url: t.url.clone(),
                })
                .collect(),
        })
        .collect();
    if let Ok(s) = serde_json::to_string_pretty(&saved) {
        let tmp = p.with_extension("json.tmp");
        if std::fs::write(&tmp, s).is_ok() {
            let _ = std::fs::rename(&tmp, &p);
        }
    }
}
