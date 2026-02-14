use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::thread;
use std::time::{Duration, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use walkdir::WalkDir;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Provider {
    Local,
    YouTube,
    SoundCloud,
    Bandcamp,
    Playlists,
}

impl Provider {
    pub const ALL: [Provider; 5] = [
        Provider::Local,
        Provider::YouTube,
        Provider::SoundCloud,
        Provider::Bandcamp,
        Provider::Playlists,
    ];

    pub fn tag(self) -> &'static str {
        match self {
            Provider::Local => "L",
            Provider::YouTube => "Y",
            Provider::SoundCloud => "S",
            Provider::Bandcamp => "B",
            Provider::Playlists => "P",
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Provider::Local => "local files",
            Provider::YouTube => "youtube",
            Provider::SoundCloud => "soundcloud",
            Provider::Bandcamp => "bandcamp",
            Provider::Playlists => "playlists",
        }
    }
}

#[derive(Clone, Debug)]
pub struct Track {
    pub title: String,
    pub artist: String,
    pub secs: f64,
    pub path: Option<PathBuf>,
    pub url: Option<String>,
    /// Lowercase "title artist", computed once so searching stays fast.
    pub key: String,
}

impl Track {
    pub fn new(title: String, artist: String, secs: f64, path: Option<PathBuf>, url: Option<String>) -> Track {
        let key = format!("{} {}", title, artist).to_lowercase();
        Track { title, artist, secs, path, url, key }
    }
}

const EXTS: [&str; 7] = ["mp3", "flac", "ogg", "wav", "m4a", "aac", "mp4"];

// ---------- library cache (skips re-reading tags for unchanged files) ----------

#[derive(Serialize, Deserialize, Clone)]
struct Entry {
    path: String,
    mtime: u64,
    size: u64,
    title: String,
    artist: String,
    secs: f64,
}

fn cache_file() -> Option<PathBuf> {
    dirs::cache_dir().map(|d| d.join("enigmatic-player").join("library.json"))
}

fn load_cache() -> HashMap<String, Entry> {
    cache_file()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|s| serde_json::from_str::<Vec<Entry>>(&s).ok())
        .map(|v| v.into_iter().map(|e| (e.path.clone(), e)).collect())
        .unwrap_or_default()
}

fn save_cache(entries: &[Entry]) {
    if let Some(p) = cache_file() {
        if let Some(parent) = p.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Ok(s) = serde_json::to_string(entries) {
            let _ = std::fs::write(p, s);
        }
    }
}

fn mtime_of(m: &std::fs::Metadata) -> u64 {
    m.modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn read_entry(p: &Path, mtime: u64, size: u64) -> Entry {
    use lofty::prelude::*;

    let mut title = p
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("untitled")
        .to_string();
    let mut artist = String::from("unknown artist");
    let mut secs = 0.0;

    if let Ok(tf) = lofty::read_from_path(p) {
        secs = tf.properties().duration().as_secs_f64();
        if let Some(tag) = tf.primary_tag().or_else(|| tf.first_tag()) {
            if let Some(t) = tag.title() {
                if !t.trim().is_empty() {
                    title = t.to_string();
                }
            }
            if let Some(a) = tag.artist() {
                if !a.trim().is_empty() {
                    artist = a.to_string();
                }
            }
        }
    }

    Entry {
        path: p.to_string_lossy().to_string(),
        mtime,
        size,
        title,
        artist,
        secs,
    }
}

fn entry_to_track(e: Entry) -> Track {
    Track::new(e.title, e.artist, e.secs, Some(PathBuf::from(e.path)), None)
}

pub fn track_from_path(p: &Path) -> Track {
    let (m, s) = std::fs::metadata(p)
        .map(|m| (mtime_of(&m), m.len()))
        .unwrap_or((0, 0));
    entry_to_track(read_entry(p, m, s))
}

/// Scan a folder recursively. Unchanged files come from the cache; new or
/// changed files are read in parallel.
pub fn scan(dir: &Path) -> Vec<Track> {
    let cache = load_cache();
    let mut have: Vec<Entry> = Vec::new();
    let mut todo: Vec<(PathBuf, u64, u64)> = Vec::new();

    for e in WalkDir::new(dir).follow_links(true).into_iter().filter_map(|e| e.ok()) {
        if !e.file_type().is_file() {
            continue;
        }
        let ok = e
            .path()
            .extension()
            .and_then(|x| x.to_str())
            .map(|x| EXTS.contains(&x.to_lowercase().as_str()))
            .unwrap_or(false);
        if !ok {
            continue;
        }
        let (mtime, size) = e
            .metadata()
            .map(|m| (mtime_of(&m), m.len()))
            .unwrap_or((0, 0));
        let key = e.path().to_string_lossy().to_string();
        match cache.get(&key) {
            Some(c) if c.mtime == mtime && c.size == size => have.push(c.clone()),
            _ => todo.push((e.path().to_path_buf(), mtime, size)),
        }
    }

    if !todo.is_empty() {
        let n = thread::available_parallelism().map(|n| n.get()).unwrap_or(2).clamp(1, 8);
        let chunk = ((todo.len() + n - 1) / n).max(1);
        let fresh: Vec<Entry> = thread::scope(|s| {
            let handles: Vec<_> = todo
                .chunks(chunk)
                .map(|c| {
                    s.spawn(move || {
                        c.iter()
                            .map(|(p, m, z)| read_entry(p, *m, *z))
                            .collect::<Vec<Entry>>()
                    })
                })
                .collect();
            handles
                .into_iter()
                .flat_map(|h| h.join().unwrap_or_default())
                .collect()
        });
        have.extend(fresh);
    }

    save_cache(&have);

    let mut out: Vec<Track> = have.into_iter().map(entry_to_track).collect();
    out.sort_by(|a, b| {
        (a.artist.to_lowercase(), a.title.to_lowercase())
            .cmp(&(b.artist.to_lowercase(), b.title.to_lowercase()))
    });
    out
}

// ---------- tools ----------

fn tool_ok(name: &str, arg: &str) -> bool {
    Command::new(name)
        .arg(arg)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

pub fn has_ytdlp() -> bool {
    tool_ok("yt-dlp", "--version")
}

pub fn has_ffmpeg() -> bool {
    tool_ok("ffmpeg", "-version")
}

pub fn last_line(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes)
        .lines()
        .rev()
        .find(|l| !l.trim().is_empty())
        .unwrap_or("")
        .trim()
        .to_string()
}

/// Errors worth retrying: rate limits, timeouts, "service busy".
pub fn is_transient(msg: &str) -> bool {
    let m = msg.to_lowercase();
    [
        "429",
        "too many requests",
        "502",
        "503",
        "504",
        "timed out",
        "timeout",
        "temporar",
        "busy",
        "connection reset",
        "connection aborted",
        "try again",
        "rate limit",
    ]
    .iter()
    .any(|k| m.contains(k))
}

fn run_search(target: &str) -> Result<Vec<Track>, String> {
    let out = Command::new("yt-dlp")
        .args([
            "--flat-playlist",
            "--no-warnings",
            "--socket-timeout",
            "10",
            "--retries",
            "2",
            "--print",
            "%(title)s\t%(uploader,channel)s\t%(duration)s\t%(webpage_url,url)s",
        ])
        .arg(target)
        .output()
        .map_err(|e| format!("yt-dlp not found: {e}"))?;

    if !out.status.success() {
        let msg = last_line(&out.stderr);
        return Err(if msg.is_empty() { "search failed".into() } else { msg });
    }

    let text = String::from_utf8_lossy(&out.stdout);
    let tracks: Vec<Track> = text
        .lines()
        .filter_map(|line| {
            let p: Vec<&str> = line.split('\t').collect();
            if p.len() < 4 || p[3] == "NA" || p[3].is_empty() {
                return None;
            }
            Some(Track::new(
                p[0].to_string(),
                if p[1] == "NA" { "unknown artist".into() } else { p[1].to_string() },
                p[2].parse::<f64>().unwrap_or(0.0),
                None,
                Some(p[3].to_string()),
            ))
        })
        .collect();

    if tracks.is_empty() {
        Err("no results".into())
    } else {
        Ok(tracks)
    }
}

/// Search YouTube or SoundCloud, or list a pasted URL. Retries when the service is busy.
pub fn search_remote(
    provider: Provider,
    query: &str,
    on_retry: &dyn Fn(usize, usize),
) -> Result<Vec<Track>, String> {
    let target = if query.starts_with("http://") || query.starts_with("https://") {
        query.to_string()
    } else {
        match provider {
            Provider::YouTube => format!("ytsearch10:{query}"),
            Provider::SoundCloud => format!("scsearch10:{query}"),
            _ => return Err("this source needs a track or album url".into()),
        }
    };

    let max = 3;
    let mut last = String::from("search failed");
    for attempt in 1..=max {
        match run_search(&target) {
            Ok(t) => return Ok(t),
            Err(e) => {
                if attempt < max && is_transient(&e) {
                    on_retry(attempt + 1, max);
                    thread::sleep(Duration::from_millis(1200 * attempt as u64));
                    last = e;
                    continue;
                }
                return Err(e);
            }
        }
    }
    Err(last)
}

fn run_download(url: &str, dir: &Path) -> Result<PathBuf, String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("cannot create folder: {e}"))?;
    let template = dir.join("%(title)s.%(ext)s");

    let out = Command::new("yt-dlp")
        .args([
            "-f",
            "bestaudio/best",
            "-x",
            "--audio-format",
            "mp3",
            "--audio-quality",
            "0",
            "--no-playlist",
            "--no-warnings",
            "--socket-timeout",
            "15",
            "--retries",
            "3",
            "--print",
            "after_move:filepath",
            "-o",
        ])
        .arg(&template)
        .arg(url)
        .output()
        .map_err(|e| format!("yt-dlp not found: {e}"))?;

    if !out.status.success() {
        let msg = last_line(&out.stderr);
        return Err(if msg.is_empty() { "download failed".into() } else { msg });
    }

    let path = PathBuf::from(last_line(&out.stdout));
    if path.exists() {
        Ok(path)
    } else {
        Err("download finished but the file was not found (is ffmpeg installed?)".into())
    }
}

/// Save a track as high-quality mp3 into `dir`. Retries when the service is busy.
pub fn download(url: &str, dir: &Path, on_retry: &dyn Fn(usize, usize)) -> Result<PathBuf, String> {
    let max = 3;
    let mut last = String::from("download failed");
    for attempt in 1..=max {
        match run_download(url, dir) {
            Ok(p) => return Ok(p),
            Err(e) => {
                if attempt < max && is_transient(&e) {
                    on_retry(attempt + 1, max);
                    thread::sleep(Duration::from_millis(1500 * attempt as u64));
                    last = e;
                    continue;
                }
                return Err(e);
            }
        }
    }
    Err(last)
}
