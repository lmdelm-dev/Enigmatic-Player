//! Streaming pipeline for online tracks:
//!
//!   yt-dlp (best audio) -> ffmpeg (decode, resample to the device rate)
//!   -> raw 16-bit PCM written to a cache file that grows while it plays.
//!
//! Playback starts after about a second of audio is ready, seeking inside
//! the downloaded part is instant, and nothing is encoded to mp3.

use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use crate::library::{is_transient, last_line};

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
                    if got == 0 {
                        if let Ok(mut e) = self.error.lock() {
                            *e = Some(msg);
                        }
                    }
                    break;
                }
            }
        }
        self.done.store(true, Ordering::Release);
    }

    fn pipeline(&self, file: &mut File, url: &str) -> Result<(), String> {
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
        let y_err = y.stderr.take();

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

        // collect yt-dlp's error text without blocking the pipe
        let err_thread = thread::spawn(move || {
            let mut text = Vec::new();
            if let Some(mut e) = y_err {
                let _ = e.read_to_end(&mut text);
            }
            text
        });

        let y = Arc::new(Mutex::new(y));
        let f = Arc::new(Mutex::new(f));
        let finished = Arc::new(AtomicBool::new(false));

        // watcher: kills both processes when the track changes
        {
            let (y, f, finished, kill) = (y.clone(), f.clone(), finished.clone(), self.kill.clone());
            thread::spawn(move || loop {
                if finished.load(Ordering::Relaxed) {
                    break;
                }
                if kill.load(Ordering::Relaxed) {
                    kill_child(&y);
                    kill_child(&f);
                    break;
                }
                thread::sleep(Duration::from_millis(100));
            });
        }

        let mut buf = vec![0u8; 65536];
        let mut first = true;
        let mut write_err: Option<String> = None;
        loop {
            let n = match out.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => n,
                Err(_) => break,
            };
            if let Err(e) = file.write_all(&buf[..n]) {
                write_err = Some(format!("cache write failed: {e}"));
                break;
            }
            self.written.fetch_add(n as u64, Ordering::Release);
            if first {
                first = false;
                self.set_note(String::new());
            }
        }

        if write_err.is_some() {
            kill_child(&y);
            kill_child(&f);
        }
        finished.store(true, Ordering::Relaxed);
        wait_child(&f);
        wait_child(&y);
        let err_text = err_thread.join().unwrap_or_default();

        if let Some(e) = write_err {
            return Err(e);
        }
        if self.written.load(Ordering::Relaxed) == 0 && !self.kill.load(Ordering::Relaxed) {
            let msg = last_line(&err_text);
            return Err(if msg.is_empty() { "no audio received".to_string() } else { msg });
        }
        Ok(())
    }
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
