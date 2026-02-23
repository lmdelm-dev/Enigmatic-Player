//! Streaming pipeline for online tracks:
//!
//!   yt-dlp (resolve the best audio url) -> ffmpeg (decode, resample to the
//!   device rate) -> raw 16-bit PCM written to a cache file that grows while
//!   it plays.
//!
//! Resolving the URL first (instead of letting yt-dlp download) skips the
//! pre-roll ad wait yt-dlp enforces on YouTube, which used to delay every
//! track by several seconds. When a site refuses a direct url, the old
//! yt-dlp downloader is used as a fallback.
//!
//! Playback starts after about a second of audio is ready, seeking inside
//! the downloaded part is instant, and nothing is encoded to mp3.

use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::PathBuf;
use std::process::{Child, ChildStderr, Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use crate::library::{is_transient, last_line};

/// Some servers (YouTube) reject ffmpeg's default user agent.
const UA: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64)";

#[derive(Clone)]
pub struct Stream {
    pub path: PathBuf,
    pub rate: u32,
    pub secs: f64,
    /// Bytes of PCM written so far.
    pub written: Arc<AtomicU64>,
    /// Samples (not frames) the player has consumed.
    pub consumed: Arc<AtomicU64>,
    pub done: Arc<AtomicBool>,
    pub kill: Arc<AtomicBool>,
    pub note: Arc<Mutex<String>>,
    pub error: Arc<Mutex<Option<String>>>,
}

pub fn cache_dir() -> PathBuf {
    dirs::cache_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("enigmatic-player")
        .join("stream")
}

pub fn clear_cache() {
    let _ = fs::remove_dir_all(cache_dir());
}

impl Stream {
    pub fn start(url: &str, rate: u32, secs: f64) -> Result<Stream, String> {
        let dir = cache_dir();
        fs::create_dir_all(&dir).map_err(|e| format!("cannot create cache folder: {e}"))?;

        let id = {
            use std::hash::{Hash, Hasher};
            let mut h = std::collections::hash_map::DefaultHasher::new();
            url.hash(&mut h);
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
                .hash(&mut h);
            h.finish()
        };
        let path = dir.join(format!("{id:x}.pcm"));
        let file = File::create(&path).map_err(|e| format!("cannot create cache file: {e}"))?;

        let s = Stream {
            path,
            rate,
            secs,
            written: Arc::new(AtomicU64::new(0)),
            consumed: Arc::new(AtomicU64::new(0)),
            done: Arc::new(AtomicBool::new(false)),
            kill: Arc::new(AtomicBool::new(false)),
            note: Arc::new(Mutex::new(String::new())),
            error: Arc::new(Mutex::new(None)),
        };

        let worker = s.clone();
        let url = url.to_string();
        thread::spawn(move || worker.run(file, url));
        Ok(s)
    }

    fn set_note(&self, text: String) {
        if let Ok(mut n) = self.note.lock() {
            *n = text;
        }
    }

    fn run(&self, mut file: File, url: String) {
        let max = 3;
        for attempt in 1..=max {
            if self.kill.load(Ordering::Relaxed) {
                break;
            }
            match self.pipeline(&mut file, &url) {
                Ok(()) => break,
                Err(msg) => {
                    let got = self.written.load(Ordering::Relaxed);
                    if got == 0 && attempt < max && is_transient(&msg) && !self.kill.load(Ordering::Relaxed) {
                        self.set_note(format!("service busy, retrying ({}/{})", attempt + 1, max));
                        // wait in small slices so a track change can cancel us
                        let wait_ms = 1500 * attempt as u64;
                        let mut waited = 0;
                        while waited < wait_ms && !self.kill.load(Ordering::Relaxed) {
                            thread::sleep(Duration::from_millis(100));
                            waited += 100;
                        }
                        continue;
                    }
                    if got == 0 && !self.kill.load(Ordering::Relaxed) {
                        if let Ok(mut e) = self.error.lock() {
                            *e = Some(msg);
                        }
                    }
                    break;
                }
            }
        }
        self.done.store(true, Ordering::Release);
        // Close the cache file before deleting it: Windows keeps it locked
        // while a handle is open. Only discarded streams delete their file;
        // finished ones stay cached until the next start or shutdown.
        drop(file);
        if self.kill.load(Ordering::Relaxed) {
            let _ = fs::remove_file(&self.path);
        }
    }

    fn pipeline(&self, file: &mut File, url: &str) -> Result<(), String> {
        let media = match self.resolve(url) {
            Ok(m) => m,
            Err(_) if self.kill.load(Ordering::Relaxed) => return Ok(()),
            Err(e) => return Err(e),
        };
        if self.kill.load(Ordering::Relaxed) {
            return Ok(());
        }
        match self.decode(file, &media) {
            Ok(()) => Ok(()),
            // The site may only serve audio through yt-dlp itself (cookies,
            // referer, signed headers). Nothing was written yet, so retry
            // with the downloader path.
            Err(_) if self.written.load(Ordering::Relaxed) == 0 && !self.kill.load(Ordering::Relaxed) => {
                self.download(file, url)
            }
            Err(e) => Err(e),
        }
    }

    /// Ask yt-dlp for the direct media url without downloading anything.
    ///
    /// YouTube's default web client gates the media url behind the pre-roll
    /// ad window: it answers 403 for several seconds after the page loads.
    /// The embedded client serves the same audio immediately, so it is tried
    /// first and the default client is kept as a fallback.
    fn resolve(&self, url: &str) -> Result<String, String> {
        let embedded = self.resolve_with(url, &["--extractor-args", "youtube:player_client=web_embedded"]);
        match embedded {
            Ok(u) => Ok(u),
            Err(e) => {
                if self.kill.load(Ordering::Relaxed) {
                    return Err(e);
                }
                self.resolve_with(url, &[])
            }
        }
    }

    fn resolve_with(&self, url: &str, extra: &[&str]) -> Result<String, String> {
        let mut y = Command::new("yt-dlp")
            .args([
                "-f",
                "bestaudio/best",
                "--no-playlist",
                "--no-warnings",
                "--quiet",
                "--no-progress",
                "--socket-timeout",
                "15",
                "--retries",
                "3",
                "--fragment-retries",
                "3",
            ])
            .args(extra)
            .arg("-g")
            .arg(url)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| format!("yt-dlp not found: {e}"))?;

        let mut out = y.stdout.take().ok_or_else(|| "no yt-dlp output".to_string())?;
        let err = drain(y.stderr.take());
        let y = Arc::new(Mutex::new(y));
        let finished = Arc::new(AtomicBool::new(false));
        watch(y.clone(), finished.clone(), self.kill.clone());

        let mut text = String::new();
        let read = out.read_to_string(&mut text);
        finished.store(true, Ordering::Relaxed);
        let status = wait_status(&y);
        let err_text = err.join().unwrap_or_default();

        if self.kill.load(Ordering::Relaxed) {
            return Err("cancelled".into());
        }
        if read.is_err() {
            return Err("could not read yt-dlp output".into());
        }
        if !status.map(|s| s.success()).unwrap_or(false) {
            let msg = last_line(&err_text);
            return Err(if msg.is_empty() {
                "yt-dlp could not resolve the stream".to_string()
            } else {
                msg
            });
        }
        text.lines()
            .map(str::trim)
            .filter(|l| l.starts_with("http"))
            .last()
            .map(|l| l.to_string())
            .ok_or_else(|| "yt-dlp returned no stream url".to_string())
    }

    /// Decode a direct media url with ffmpeg into the growing PCM cache file.
    fn decode(&self, file: &mut File, media: &str) -> Result<(), String> {
        let mut f = Command::new("ffmpeg")
            .args([
                "-hide_banner",
                "-loglevel",
                "error",
                "-nostdin",
                "-user_agent",
                UA,
                "-reconnect",
                "1",
                "-reconnect_streamed",
                "1",
                "-reconnect_delay_max",
                "5",
                "-probesize",
                "131072",
                "-analyzeduration",
                "500000",
                "-i",
                media,
                "-vn",
                "-f",
                "s16le",
                "-acodec",
                "pcm_s16le",
                "-ac",
                "2",
                "-ar",
            ])
            .arg(self.rate.to_string())
            .arg("pipe:1")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| format!("ffmpeg not found: {e}"))?;

        let mut out = f.stdout.take().ok_or_else(|| "no ffmpeg output".to_string())?;
        let err = drain(f.stderr.take());
        let f = Arc::new(Mutex::new(f));
        let finished = Arc::new(AtomicBool::new(false));
        watch(f.clone(), finished.clone(), self.kill.clone());

        let write_err = self.pump(&mut out, file);

        if write_err.is_some() {
            kill_child(&f);
        }
        finished.store(true, Ordering::Relaxed);
        wait_child(&f);
        let err_text = err.join().unwrap_or_default();

        if let Some(e) = write_err {
            return Err(e);
        }
        if self.written.load(Ordering::Relaxed) == 0 && !self.kill.load(Ordering::Relaxed) {
            let msg = last_line(&err_text);
            return Err(if msg.is_empty() {
                "no audio received".to_string()
            } else {
                msg
            });
        }
        Ok(())
    }

    /// Fallback: let yt-dlp download the audio and pipe it through ffmpeg.
    /// Slower (it waits out YouTube's ad delay) but works with signed urls.
    fn download(&self, file: &mut File, url: &str) -> Result<(), String> {
        let mut y = Command::new("yt-dlp")
            .args([
                "-f",
                "bestaudio/best",
                "--no-playlist",
                "--no-warnings",
                "--quiet",
                "--no-progress",
                "--socket-timeout",
                "15",
                "--retries",
                "3",
                "--fragment-retries",
                "3",
                "--concurrent-fragments",
                "4",
                "-o",
                "-",
            ])
            .arg(url)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| format!("yt-dlp not found: {e}"))?;

        let y_out = y.stdout.take().ok_or_else(|| "no yt-dlp output".to_string())?;
        let err = drain(y.stderr.take());

        let mut f = match Command::new("ffmpeg")
            .args([
                "-hide_banner",
                "-loglevel",
                "error",
                "-nostdin",
                "-probesize",
                "131072",
                "-analyzeduration",
                "500000",
                "-i",
                "pipe:0",
                "-vn",
                "-f",
                "s16le",
                "-acodec",
                "pcm_s16le",
                "-ac",
                "2",
                "-ar",
            ])
            .arg(self.rate.to_string())
            .arg("pipe:1")
            .stdin(Stdio::from(y_out))
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
        {
            Ok(c) => c,
            Err(e) => {
                let _ = y.kill();
                let _ = y.wait();
                return Err(format!("ffmpeg not found: {e}"));
            }
        };

        let mut out = f.stdout.take().ok_or_else(|| "no ffmpeg output".to_string())?;

        let y = Arc::new(Mutex::new(y));
        let f = Arc::new(Mutex::new(f));
        let finished = Arc::new(AtomicBool::new(false));
        watch(y.clone(), finished.clone(), self.kill.clone());
        watch(f.clone(), finished.clone(), self.kill.clone());

        let write_err = self.pump(&mut out, file);

        if write_err.is_some() {
            kill_child(&y);
            kill_child(&f);
        }
        finished.store(true, Ordering::Relaxed);
        wait_child(&f);
        wait_child(&y);
        let err_text = err.join().unwrap_or_default();

        if let Some(e) = write_err {
            return Err(e);
        }
        if self.written.load(Ordering::Relaxed) == 0 && !self.kill.load(Ordering::Relaxed) {
            let msg = last_line(&err_text);
            return Err(if msg.is_empty() {
                "no audio received".to_string()
            } else {
                msg
            });
        }
        Ok(())
    }

    /// Copy a child's stdout into the cache file, counting the bytes written.
    fn pump(&self, out: &mut impl Read, file: &mut File) -> Option<String> {
        let mut buf = vec![0u8; 65536];
        let mut first = true;
        loop {
            let n = match out.read(&mut buf) {
                Ok(0) => return None,
                Ok(n) => n,
                Err(_) => return None,
            };
            if let Err(e) = file.write_all(&buf[..n]) {
                return Some(format!("cache write failed: {e}"));
            }
            self.written.fetch_add(n as u64, Ordering::Release);
            if first {
                first = false;
                self.set_note(String::new());
            }
        }
    }
}

/// Reads a child's stderr on its own thread so a full pipe can never block it.
fn drain(stderr: Option<ChildStderr>) -> thread::JoinHandle<Vec<u8>> {
    thread::spawn(move || {
        let mut text = Vec::new();
        if let Some(mut e) = stderr {
            let _ = e.read_to_end(&mut text);
        }
        text
    })
}

/// Kills `child` when the track changes, and stops once `finished` is set.
fn watch(child: Arc<Mutex<Child>>, finished: Arc<AtomicBool>, kill: Arc<AtomicBool>) {
    thread::spawn(move || loop {
        if finished.load(Ordering::Relaxed) {
            break;
        }
        if kill.load(Ordering::Relaxed) {
            kill_child(&child);
            break;
        }
        thread::sleep(Duration::from_millis(100));
    });
}

fn kill_child(c: &Arc<Mutex<Child>>) {
    if let Ok(mut ch) = c.lock() {
        let _ = ch.kill();
    }
}

fn wait_child(c: &Arc<Mutex<Child>>) {
    if let Ok(mut ch) = c.lock() {
        let _ = ch.wait();
    }
}

fn wait_status(c: &Arc<Mutex<Child>>) -> Option<ExitStatus> {
    c.lock().ok().and_then(|mut ch| ch.wait().ok())
}


