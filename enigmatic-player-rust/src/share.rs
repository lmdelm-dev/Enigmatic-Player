use std::io::Write;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::library::Track;
use crate::playlists::Playlist;

/// A track as it travels in a share code or a listening room. Local file
/// paths never leave the machine: only the file name is included as a hint,
/// so another user can match the track against their own library.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct SharedTrack {
    pub title: String,
    pub artist: String,
    pub secs: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file: Option<String>,
}

/// Strips control characters. Shared text is rendered by a terminal, so a
/// pasted tag must never be able to inject escape sequences.
pub fn sanitize(s: &str) -> String {
    s.chars().filter(|c| !c.is_control()).collect()
}

/// Only plain web URLs with no whitespace. This feeds yt-dlp as an argument,
/// so anything that starts with a dash must never come from the network.
pub fn url_ok(url: &str) -> bool {
    let u = url.trim();
    let no_space = !u.chars().any(char::is_whitespace);
    let scheme = u.starts_with("http://") || u.starts_with("https://");
    no_space && scheme && !u.chars().any(char::is_control)
}

impl SharedTrack {
    pub fn from_track(t: &Track) -> SharedTrack {
        SharedTrack {
            title: sanitize(&t.title),
            artist: sanitize(&t.artist),
            secs: t.secs,
            url: t.url.clone().filter(|u| url_ok(u)),
            file: t
                .path
                .as_ref()
                .and_then(|p| p.file_name())
                .map(|n| sanitize(&n.to_string_lossy())),
        }
    }

    /// An unplayable placeholder for a track this machine does not have.
    pub fn to_track(&self) -> Track {
        Track::new(
            self.title.clone(),
            self.artist.clone(),
            self.secs,
            None,
            self.url.clone(),
        )
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct SharedList {
    pub name: String,
    pub tracks: Vec<SharedTrack>,
}

impl SharedList {
    pub fn from_playlist(p: &Playlist) -> SharedList {
        SharedList {
            name: p.name.clone(),
            tracks: p.tracks.iter().map(SharedTrack::from_track).collect(),
        }
    }

    /// Resolves every shared track against the local library. Local tracks are
    /// matched by title + artist first, then by file name. Anything unmatched
    /// stays as a metadata-only track so the playlist is still complete.
    pub fn resolve(&self, local: &[Track]) -> Vec<Track> {
        self.tracks.iter().map(|s| resolve_track(s, local)).collect()
    }
}

pub fn resolve_track(s: &SharedTrack, local: &[Track]) -> Track {
    if s.url.is_some() {
        return s.to_track();
    }
    let title = s.title.to_lowercase();
    let artist = s.artist.to_lowercase();
    if let Some(t) = local
        .iter()
        .find(|t| t.title.to_lowercase() == title && t.artist.to_lowercase() == artist)
    {
        return t.clone();
    }
    if let Some(file) = &s.file {
        let file = file.to_lowercase();
        if let Some(t) = local.iter().find(|t| {
            t.path
                .as_ref()
                .and_then(|p| p.file_name())
                .map(|n| n.to_string_lossy().to_lowercase() == file)
                .unwrap_or(false)
        }) {
            return t.clone();
        }
    }
    s.to_track()
}

const LIST_PREFIX: &str = "ELIST1:";

pub fn encode_playlist(p: &Playlist) -> Result<String, String> {
    let list = SharedList::from_playlist(p);
    let json = serde_json::to_vec(&list).map_err(|e| e.to_string())?;
    Ok(format!("{LIST_PREFIX}{}", b64(&json, false)))
}

pub fn decode_playlist(code: &str) -> Result<SharedList, String> {
    let code = code.trim();
    let body = code
        .strip_prefix(LIST_PREFIX)
        .or_else(|| code.strip_prefix("elist1:"))
        .ok_or_else(|| "not a playlist share code".to_string())?;
    let bytes = unb64(body)?;
    serde_json::from_slice(&bytes).map_err(|e| format!("invalid playlist share code: {e}"))
}

fn safe_name(name: &str) -> String {
    let mut out: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == ' ' {
                c
            } else {
                '_'
            }
        })
        .collect();
    out = out.trim().to_string();
    if out.is_empty() {
        out = "playlist".into();
    }
    out.chars().take(48).collect()
}

/// Writes the share code to a text file the user can send to someone else.
pub fn save_code(dir: &Path, name: &str, code: &str) -> Option<PathBuf> {
    std::fs::create_dir_all(dir).ok()?;
    let path = dir.join(format!("{}.elist.txt", safe_name(name)));
    std::fs::write(&path, format!("{code}\n")).ok()?;
    Some(path)
}

/// Sends text to the terminal clipboard with OSC 52. Many terminals support
/// this and some ignore it, so callers should also offer the saved file.
pub fn copy_to_terminal(text: &str) -> bool {
    let seq = format!("\x1b]52;c;{}\x07", b64(text.as_bytes(), true));
    let mut out = std::io::stdout();
    out.write_all(seq.as_bytes()).is_ok() && out.flush().is_ok()
}

const URL_ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
const STD_ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

fn b64(data: &[u8], standard: bool) -> String {
    let alpha = if standard { STD_ALPHABET } else { URL_ALPHABET };
    let mut out = String::with_capacity((data.len() + 2) / 3 * 4);
    for chunk in data.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = *chunk.get(1).unwrap_or(&0) as u32;
        let b2 = *chunk.get(2).unwrap_or(&0) as u32;
        let n = (b0 << 16) | (b1 << 8) | b2;
        out.push(alpha[(n >> 18) as usize & 63] as char);
        out.push(alpha[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 {
            alpha[(n >> 6) as usize & 63] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            alpha[n as usize & 63] as char
        } else {
            '='
        });
    }
    out
}

fn unb64(s: &str) -> Result<Vec<u8>, String> {
    let mut vals: Vec<u8> = Vec::with_capacity(s.len());
    for c in s.bytes() {
        if c == b'=' || c.is_ascii_whitespace() {
            continue;
        }
        let v = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' | b'-' => 62,
            b'/' | b'_' => 63,
            _ => return Err("share code contains an invalid character".into()),
        };
        vals.push(v);
    }
    if vals.len() % 4 == 1 {
        return Err("share code is cut off".into());
    }
    let mut out = Vec::with_capacity(vals.len() / 4 * 3);
    for chunk in vals.chunks(4) {
        let mut n: u32 = 0;
        for (i, v) in chunk.iter().enumerate() {
            n |= (*v as u32) << (18 - i * 6);
        }
        out.push((n >> 16) as u8);
        if chunk.len() > 2 {
            out.push((n >> 8) as u8);
        }
        if chunk.len() > 3 {
            out.push(n as u8);
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn track(title: &str, artist: &str, path: Option<&str>, url: Option<&str>) -> Track {
        Track::new(
            title.into(),
            artist.into(),
            12.5,
            path.map(PathBuf::from),
            url.map(|u| u.to_string()),
        )
    }

    #[test]
    fn base64_roundtrip() {
        for data in [b"".as_slice(), b"a", b"ab", b"abc", b"hello world!"] {
            assert_eq!(unb64(&b64(data, false)).unwrap(), data);
        }
    }

    #[test]
    fn playlist_code_roundtrip_and_no_paths() {
        let p = Playlist {
            name: "Road trip".into(),
            tracks: vec![
                track("One", "Artist", Some("C:\\music\\secret\\one.mp3"), None),
                track("Two", "Online", None, Some("https://example.com/two")),
                track("Evil", "Tag\x1b[31minject", None, Some("ytdlp://bad")),
            ],
        };
        let code = encode_playlist(&p).unwrap();
        assert!(code.starts_with("ELIST1:"));
        assert!(!code.contains("C:\\music"));
        assert!(!code.contains("secret"));
        assert!(!code.contains('\x1b'));
        let back = decode_playlist(&code).unwrap();
        assert_eq!(back.name, "Road trip");
        assert_eq!(back.tracks.len(), 3);
        assert_eq!(back.tracks[0].file.as_deref(), Some("one.mp3"));
        assert_eq!(back.tracks[1].url.as_deref(), Some("https://example.com/two"));
        // a non-http url and control characters must not survive a share code
        assert_eq!(back.tracks[2].url, None);
        assert_eq!(back.tracks[2].artist, "Tag[31minject");
        assert!(!back.tracks[2].artist.contains('\u{1b}'));
    }

    #[test]
    fn url_check_rejects_bad_schemes() {
        assert!(url_ok("https://example.com/a.mp3"));
        assert!(url_ok("http://example.com/x"));
        assert!(!url_ok("ytdlp://bad"));
        assert!(!url_ok("--exec blah"));
        assert!(!url_ok("https://a b/c"));
        assert!(!url_ok(""));
    }

    #[test]
    fn playlist_code_rejects_junk() {
        assert!(decode_playlist("hello").is_err());
        assert!(decode_playlist("ELIST1:!!!!").is_err());
    }

    #[test]
    fn saved_share_file_roundtrip() {
        let p = Playlist {
            name: "share file deal".into(),
            tracks: vec![track("Song", "Band", Some("C:\\m\\s.mp3"), None)],
        };
        let code = encode_playlist(&p).unwrap();
        let dir = std::env::temp_dir().join("enigmatic-player-test-shares");
        let path = save_code(&dir, &p.name, &code).unwrap();
        assert!(path.exists());
        let as_saved = std::fs::read_to_string(&path).unwrap();
        let back = decode_playlist(&as_saved).unwrap();
        assert_eq!(back.name, "share file deal");
        assert_eq!(back.tracks[0].title, "Song");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn matches_local_track_by_tags_then_file_name() {
        let local = vec![
            track("Song", "Band", Some("/music/a.mp3"), None),
            track("Other", "Someone", Some("/music/b-file.mp3"), None),
        ];
        let by_tags = SharedTrack {
            title: "song".into(),
            artist: "band".into(),
            secs: 12.5,
            url: None,
            file: Some("renamed.mp3".into()),
        };
        assert_eq!(resolve_track(&by_tags, &local).path.as_deref(), Some(Path::new("/music/a.mp3")));

        let by_file = SharedTrack {
            title: "not the tag".into(),
            artist: "nobody".into(),
            secs: 12.5,
            url: None,
            file: Some("B-FILE.MP3".into()),
        };
        assert_eq!(resolve_track(&by_file, &local).path.as_deref(), Some(Path::new("/music/b-file.mp3")));

        let missing = SharedTrack {
            title: "gone".into(),
            artist: "nobody".into(),
            secs: 0.0,
            url: None,
            file: None,
        };
        let t = resolve_track(&missing, &local);
        assert!(t.path.is_none() && t.url.is_none());
    }
}
