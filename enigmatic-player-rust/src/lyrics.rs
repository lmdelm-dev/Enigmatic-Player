//! Lyrics for the playing track.
//!
//! Lookup order: a sidecar file next to the audio file (`.lrc` / `.txt`),
//! lyrics embedded in the audio tags, a cached copy, then the LRCLIB
//! community database. Everything runs on a worker thread; the UI only ever
//! sees a small [`State`] value.

use std::fs;
use std::path::PathBuf;
use std::sync::Once;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::library::Track;

const LRCLIB_GET: &str = "https://lrclib.net/api/get";
const LRCLIB_SEARCH: &str = "https://lrclib.net/api/search";
const UA: &str = "enigmatic-player/1.8 (terminal music player)";

/// One displayed lyric line. `at` is the playback time in seconds when the
/// line becomes current; `None` for unsynced (plain text) lyrics.
#[derive(Clone, Debug, PartialEq)]
pub struct Line {
    pub at: Option<f64>,
    pub text: String,
}

#[derive(Clone, Debug, Default)]
pub struct Lyrics {
    pub lines: Vec<Line>,
    /// Where the lyrics came from, for the popup footer.
    pub source: &'static str,
    pub instrumental: bool,
}

impl Lyrics {
    pub fn synced(&self) -> bool {
        self.lines.iter().any(|l| l.at.is_some())
    }

    /// Index of the line that should be highlighted at playback time `t`.
    pub fn current(&self, t: f64) -> Option<usize> {
        if !self.synced() {
            return None;
        }
        let t = t + 0.05;
        let mut lo = 0usize;
        let mut hi = self.lines.len();
        while lo < hi {
            let mid = (lo + hi) / 2;
            match self.lines[mid].at {
                Some(at) if at <= t => lo = mid + 1,
                _ => hi = mid,
            }
        }
        if lo == 0 {
            None
        } else {
            Some(lo - 1)
        }
    }
}

/// What the UI knows about the current track's lyrics.
#[derive(Clone, Debug, Default)]
pub enum State {
    #[default]
    Idle,
    Loading,
    Ready(Lyrics),
    Missing(String),
}

/// Stable cache key for a track. Duration keeps different versions of the
/// same song apart (live, remaster, album cut ...).
pub fn key(t: &Track) -> String {
    let artist = slug(&t.artist);
    let title = slug(&t.title);
    let artist = if artist.is_empty() { "unknown".to_string() } else { artist };
    let title = if title.is_empty() { "untitled".to_string() } else { title };
    format!("{}-{}-{}", artist, title, t.secs.round().max(0.0) as i64)
}

/// True when online lookup is allowed. Set `ENIGMATIC_NO_LYRICS_ONLINE=1`
/// to keep the player fully local.
pub fn online_enabled() -> bool {
    std::env::var_os("ENIGMATIC_NO_LYRICS_ONLINE").is_none()
}

/// Load lyrics for `track`, blocking until done. Called on a worker thread.
pub fn load(track: &Track) -> Result<Lyrics, String> {
    // 1. a sidecar file next to the song
    if let Some(path) = &track.path {
        for ext in ["lrc", "LRC", "txt", "TXT"] {
            let sidecar = path.with_extension(ext);
            if sidecar == *path {
                continue;
            }
            if let Ok(raw) = fs::read_to_string(&sidecar) {
                if let Some(lyrics) = parse(&raw, "file", false) {
                    return Ok(lyrics);
                }
            }
        }

        // 2. lyrics embedded in the file's tags
        if let Some(raw) = embedded(path) {
            if let Some(lyrics) = parse(&raw, "tags", false) {
                return Ok(lyrics);
            }
        }
    }

    // 3. a copy downloaded earlier
    if let Some(lyrics) = cache_load(track) {
        return Ok(lyrics);
    }

    // 4. the LRCLIB database
    if !online_enabled() {
        return Err("no lyrics found (online lookup is off)".into());
    }
    let hit = fetch(track)?;
    save_cache(track, &hit);
    from_hit(&hit, "lrclib").ok_or_else(|| {
        format!("no lyrics found for {} - {}", track.title, track.artist)
    })
}

fn slug(s: &str) -> String {
    let mut out = String::new();
    for c in s.chars() {
        if out.len() >= 48 {
            break;
        }
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
        } else if !out.is_empty() && !out.ends_with('-') {
            out.push('-');
        }
    }
    while out.ends_with('-') {
        out.pop();
    }
    out
}

fn embedded(path: &std::path::Path) -> Option<String> {
    use lofty::prelude::*;
    let tagged = lofty::read_from_path(path).ok()?;
    let tag = tagged.primary_tag().or_else(|| tagged.first_tag())?;
    let text = tag.get_string(&lofty::tag::ItemKey::Lyrics)?;
    if text.trim().is_empty() {
        None
    } else {
        Some(text.to_string())
    }
}

// ---------- parsing ----------

/// Parse LRC or plain text lyrics. Returns `None` when there is nothing
/// usable in `raw`.
pub fn parse(raw: &str, source: &'static str, instrumental: bool) -> Option<Lyrics> {
    let mut offset_ms = 0i64;
    let mut timed: Vec<Line> = Vec::new();
    let mut plain: Vec<Line> = Vec::new();

    for raw_line in raw.lines() {
        let raw_line = raw_line.trim_end_matches('\r');
        let mut rest = raw_line.trim_start();
        let mut stamps: Vec<f64> = Vec::new();
        let mut meta = false;

        // A line may carry several timestamps: [00:12.00][00:15.20]text
        loop {
            let Some(after_open) = rest.strip_prefix('[') else {
                break;
            };
            let Some(close) = after_open.find(']') else {
                break;
            };
            let inner = &after_open[..close];
            let after = &after_open[close + 1..];

            if let Some(at) = stamp(inner) {
                stamps.push(at);
                rest = after;
                continue;
            }

            let name = inner.split(':').next().unwrap_or("").trim().to_ascii_lowercase();
            let is_meta = matches!(
                name.as_str(),
                "ar" | "ti" | "al" | "by" | "offset" | "length" | "re" | "ve"
            );
            if !is_meta {
                break;
            }
            if name == "offset" {
                if let Some((_, value)) = inner.split_once(':') {
                    if let Ok(ms) = value.trim().parse::<i64>() {
                        offset_ms = ms;
                    }
                }
            }
            meta = true;
            rest = after;
        }

        if stamps.is_empty() {
            if meta {
                continue;
            }
            let text = rest.trim();
            if !text.is_empty() {
                plain.push(Line { at: None, text: text.to_string() });
            }
            continue;
        }

        let text = rest.trim().to_string();
        for at in stamps {
            timed.push(Line {
                at: Some((at + offset_ms as f64 / 1000.0).max(0.0)),
                text: text.clone(),
            });
        }
    }

    if !timed.is_empty() {
        timed.sort_by(|a, b| {
            a.at.partial_cmp(&b.at).unwrap_or(std::cmp::Ordering::Equal)
        });
        return Some(Lyrics { lines: timed, source, instrumental });
    }

    if instrumental || !plain.is_empty() {
        return Some(Lyrics { lines: tidy(plain), source, instrumental });
    }
    None
}

/// Parse one LRC timestamp: `mm:ss`, `mm:ss.xx`, `mm:ss.xxx`, `mm:ss:xx`.
fn stamp(inner: &str) -> Option<f64> {
    let s = inner.trim();
    let (minutes, rest) = s.split_once(':')?;
    if minutes.is_empty() || !minutes.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let minutes: u32 = minutes.parse().ok()?;
    let cut = rest.find(|c: char| c == '.' || c == ':').unwrap_or(rest.len());
    let (seconds, frac) = rest.split_at(cut);
    if seconds.is_empty() || !seconds.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let seconds: u32 = seconds.parse().ok()?;
    let frac = frac.trim_start_matches(['.', ':']);
    let fraction = if frac.is_empty() {
        0.0
    } else {
        let digits: String = frac.chars().take(3).collect();
        let value: u32 = digits.parse().ok()?;
        value as f64 / 10f64.powi(digits.len() as i32)
    };
    Some(minutes as f64 * 60.0 + seconds as f64 + fraction)
}

/// Drop leading/trailing blank lines and collapse runs of blanks.
fn tidy(lines: Vec<Line>) -> Vec<Line> {
    let mut out: Vec<Line> = Vec::new();
    for line in lines {
        if line.text.is_empty()
            && (out.is_empty() || out.last().map(|l| l.text.is_empty()).unwrap_or(true))
        {
            continue;
        }
        out.push(line);
    }
    while out.last().map(|l| l.text.is_empty()).unwrap_or(false) {
        out.pop();
    }
    out
}

// ---------- cache ----------

#[derive(Serialize, Deserialize)]
struct Cached {
    #[serde(default)]
    synced: Option<String>,
    #[serde(default)]
    plain: Option<String>,
    #[serde(default)]
    instrumental: bool,
}

fn cache_path(t: &Track) -> Option<PathBuf> {
    dirs::cache_dir().map(|d| {
        d.join("enigmatic-player")
            .join("lyrics")
            .join(format!("{}.json", key(t)))
    })
}

fn cache_load(t: &Track) -> Option<Lyrics> {
    let path = cache_path(t)?;
    let raw = fs::read_to_string(path).ok()?;
    let cached: Cached = serde_json::from_str(&raw).ok()?;

    if let Some(text) = cached.synced.as_deref() {
        if let Some(lyrics) = parse(text, "cache", false) {
            return Some(lyrics);
        }
    }
    if let Some(text) = cached.plain.as_deref() {
        if let Some(lyrics) = parse(text, "cache", cached.instrumental) {
            return Some(lyrics);
        }
    }
    if cached.instrumental {
        return Some(Lyrics { lines: Vec::new(), source: "cache", instrumental: true });
    }
    None
}

fn save_cache(t: &Track, hit: &Hit) {
    let Some(path) = cache_path(t) else {
        return;
    };
    let Some(parent) = path.parent() else {
        return;
    };
    if fs::create_dir_all(parent).is_err() {
        return;
    }
    let cached = Cached {
        synced: hit.synced_lyrics.clone(),
        plain: hit.plain_lyrics.clone(),
        instrumental: hit.instrumental,
    };
    if let Ok(raw) = serde_json::to_string(&cached) {
        let _ = fs::write(path, raw);
    }
}

// ---------- LRCLIB ----------

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Hit {
    #[serde(default)]
    track_name: String,
    #[serde(default)]
    artist_name: String,
    #[serde(default)]
    plain_lyrics: Option<String>,
    #[serde(default)]
    synced_lyrics: Option<String>,
    #[serde(default)]
    instrumental: bool,
}

fn from_hit(hit: &Hit, source: &'static str) -> Option<Lyrics> {
    if hit.instrumental {
        return Some(Lyrics { lines: Vec::new(), source, instrumental: true });
    }
    if let Some(text) = hit.synced_lyrics.as_deref() {
        if let Some(lyrics) = parse(text, source, false) {
            return Some(lyrics);
        }
    }
    if let Some(text) = hit.plain_lyrics.as_deref() {
        if let Some(lyrics) = parse(text, source, false) {
            return Some(lyrics);
        }
    }
    None
}

fn client() -> Result<reqwest::blocking::Client, String> {
    // reqwest is built without a TLS crypto provider so we can reuse the
    // `ring` backend already pulled in by the rest of the player.
    static INIT: Once = Once::new();
    INIT.call_once(|| {
        let _ = rustls::crypto::ring::default_provider().install_default();
    });
    reqwest::blocking::Client::builder()
        .user_agent(UA)
        .timeout(Duration::from_secs(8))
        .build()
        .map_err(|e| format!("lyrics lookup unavailable: {e}"))
}

fn fetch(track: &Track) -> Result<Hit, String> {
    let client = client()?;

    let mut query: Vec<(&str, String)> = vec![
        ("artist_name", track.artist.clone()),
        ("track_name", track.title.clone()),
    ];
    if track.secs > 1.0 {
        query.push(("duration", format!("{:.0}", track.secs)));
    }

    let response = client
        .get(LRCLIB_GET)
        .query(&query)
        .send()
        .map_err(|e| format!("lyrics lookup failed: {e}"))?;

    if response.status().is_success() {
        let body = response
            .text()
            .map_err(|e| format!("lyrics response was unreadable: {e}"))?;
        return serde_json::from_str::<Hit>(&body)
            .map_err(|e| format!("lyrics response was unreadable: {e}"));
    }
    if response.status().as_u16() != 404 {
        return Err(format!(
            "lyrics service answered HTTP {}",
            response.status().as_u16()
        ));
    }

    // Exact match failed (often a duration mismatch); search instead.
    let response = client
        .get(LRCLIB_SEARCH)
        .query(&query[..2])
        .send()
        .map_err(|e| format!("lyrics search failed: {e}"))?;
    if !response.status().is_success() {
        return Err(format!(
            "lyrics search answered HTTP {}",
            response.status().as_u16()
        ));
    }
    let body = response
        .text()
        .map_err(|e| format!("lyrics response was unreadable: {e}"))?;
    let hits: Vec<Hit> = serde_json::from_str(&body)
        .map_err(|e| format!("lyrics response was unreadable: {e}"))?;

    pick(track, hits).ok_or_else(|| {
        format!("no lyrics found for {} - {}", track.title, track.artist)
    })
}

/// Choose the best search result: prefer an exact-ish title/artist with
/// synced lyrics, then any result that actually carries lyrics.
fn pick(track: &Track, hits: Vec<Hit>) -> Option<Hit> {
    let want_title = track.title.to_lowercase();
    let want_artist = track.artist.to_lowercase();

    let usable = |h: &Hit| {
        h.instrumental
            || h.synced_lyrics.as_deref().map(|s| !s.trim().is_empty()).unwrap_or(false)
            || h.plain_lyrics.as_deref().map(|s| !s.trim().is_empty()).unwrap_or(false)
    };

    let mut best: Option<(u8, Hit)> = None;
    for hit in hits.into_iter().filter(usable) {
        let title = hit.track_name.to_lowercase();
        let artist = hit.artist_name.to_lowercase();
        let mut score = 0u8;
        if title == want_title {
            score += 4;
        } else if title.contains(&want_title) || want_title.contains(&title) {
            score += 2;
        }
        if artist == want_artist {
            score += 4;
        } else if artist.contains(&want_artist) || want_artist.contains(&artist) {
            score += 2;
        }
        if hit.synced_lyrics.as_deref().map(|s| !s.trim().is_empty()).unwrap_or(false) {
            score += 1;
        }
        if best.as_ref().map(|(s, _)| score > *s).unwrap_or(true) {
            best = Some((score, hit));
        }
    }
    best.map(|(_, hit)| hit)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(lyrics: &Lyrics, i: usize) -> f64 {
        lyrics.lines[i].at.unwrap()
    }

    #[test]
    fn parses_lrc_with_metadata_and_offset() {
        let raw = "[ar:Radiohead]\n[ti:Creep]\n[offset:+500]\n\
                   [00:19.16]When you were here before\n\
                   [00:24.09]Couldn't look you in the eye\n";
        let lyrics = parse(raw, "test", false).unwrap();
        assert_eq!(lyrics.lines.len(), 2);
        assert!((at(&lyrics, 0) - 19.66).abs() < 1e-9);
        assert!((at(&lyrics, 1) - 24.59).abs() < 1e-9);
        assert_eq!(lyrics.lines[0].text, "When you were here before");
        assert!(lyrics.synced());
    }

    #[test]
    fn parses_multiple_timestamps_and_precision() {
        let raw = "[00:12][00:15.2]again\n[01:02.345][01:05:5]later\n";
        let lyrics = parse(raw, "test", false).unwrap();
        assert_eq!(lyrics.lines.len(), 4);
        assert!((at(&lyrics, 0) - 12.0).abs() < 1e-9);
        assert!((at(&lyrics, 1) - 15.2).abs() < 1e-9);
        assert!((at(&lyrics, 2) - 62.345).abs() < 1e-9);
        assert!((at(&lyrics, 3) - 65.5).abs() < 1e-9);
    }

    #[test]
    fn sorts_out_of_order_timestamps() {
        let raw = "[00:20]second\n[00:10]first\n[00:30]third\n";
        let lyrics = parse(raw, "test", false).unwrap();
        assert_eq!(lyrics.lines[0].text, "first");
        assert_eq!(lyrics.lines[1].text, "second");
        assert_eq!(lyrics.lines[2].text, "third");
    }

    #[test]
    fn parses_plain_lyrics_and_skips_metadata() {
        let raw = "[ar:Nobody]\nfirst line\n\n\nsecond line\n\n";
        let lyrics = parse(raw, "test", false).unwrap();
        assert!(!lyrics.synced());
        assert_eq!(lyrics.lines.len(), 2);
        assert_eq!(lyrics.lines[1].text, "second line");
    }

    #[test]
    fn current_line_follows_playback() {
        let raw = "[00:00]one\n[00:10]two\n[00:20]three\n";
        let lyrics = parse(raw, "test", false).unwrap();
        assert_eq!(lyrics.current(-1.0), None);
        assert_eq!(lyrics.current(0.0), Some(0));
        assert_eq!(lyrics.current(9.0), Some(0));
        assert_eq!(lyrics.current(10.0), Some(1));
        assert_eq!(lyrics.current(100.0), Some(2));
    }

    #[test]
    fn plain_lyrics_have_no_current_line() {
        let lyrics = parse("one\ntwo\n", "test", false).unwrap();
        assert_eq!(lyrics.current(10.0), None);
    }

    #[test]
    fn key_is_stable_and_readable() {
        let track = Track::new(
            "Creep (Live)".into(),
            "Radiohead".into(),
            239.4,
            None,
            None,
        );
        assert_eq!(key(&track), "radiohead-creep-live-239");
    }

    #[test]
    fn empty_input_is_not_lyrics() {
        assert!(parse("", "test", false).is_none());
        assert!(parse("[ar:Artist]\n[ti:Song]\n", "test", false).is_none());
    }

    #[test]
    #[ignore = "needs internet: queries lrclib.net"]
    fn lrclib_lookup_returns_synced_lyrics() {
        let track = Track::new("Creep".into(), "Radiohead".into(), 239.0, None, None);
        let hit = fetch(&track).expect("lrclib lookup");
        let lyrics = from_hit(&hit, "lrclib").expect("lyrics in response");
        assert!(lyrics.synced());
        assert!(lyrics.lines.len() > 5);
    }
}
