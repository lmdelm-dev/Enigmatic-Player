use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::Ordering;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::thread;

use ratatui::{
    crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind},
    layout::Rect,
    widgets::ListState,
};

use crate::audio::{self, Engine};
use crate::beacon;
use crate::config::{expand, Config};
use crate::library::{self, Provider, Track};
use crate::playlists::{self, Playlist};
use crate::punch;
use crate::share::{self, SharedTrack};
use crate::stream::{self, Stream};
use crate::themes::{self, THEMES};
use crate::together::{self, Event, RoomHandle, RoomState};

use std::net::TcpListener;

pub const REVERBS: [(&str, u32); 5] = [("off", 0), ("room", 25), ("hall", 45), ("cave", 65), ("space", 85)];
pub const SPEEDS: [f32; 16] = [
    0.5, 0.6, 0.7, 0.8, 0.9, 1.0, 1.1, 1.2, 1.3, 1.4, 1.5, 1.6, 1.7, 1.8, 1.9, 2.0,
];
pub const NB_VOL: usize = 16;
pub const NB_RV: usize = 8;
pub const BANDS: usize = 14;
/// Index of the playlists source in Provider::ALL.
pub const PLAYLISTS: usize = 4;
pub const PICKS: [&str; 4] = [
    "~/Music/enigmatic-player",
    "~/Music",
    "~/Downloads/music",
    "/mnt/media/music",
];

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Normal,
    Search,
    Settings,
    Pick,
    Name,
    Share,
    Join,
    Room,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Btn {
    Prev,
    Back,
    Play,
    Fwd,
    Next,
}

type SearchKey = (usize, String);

pub enum Msg {
    Scanned(Vec<Track>),
    Found { seq: u64, key: SearchKey, res: Result<Vec<Track>, String> },
    Note(u64, String),
    Saved(Result<Track, String>),
    Together(Event),
    /// The result of the automatic router port-opening run in the background.
    Reach(punch::Reach),
}

/// Clickable areas, refreshed on every draw.
#[derive(Default)]
pub struct Hit {
    pub progress: Rect,
    pub vol: Rect,
    pub mute: Rect,
    pub rv_bar: Rect,
    pub gear: Rect,
    pub lib_header: Rect,
    pub tracks: Rect,
    pub tracks_offset: usize,
    pub btns: Vec<(Rect, Btn)>,
    pub speeds: Vec<(Rect, f32)>,
    pub reverbs: Vec<(Rect, usize)>,
    pub sources: Vec<(Rect, usize)>,
}

pub struct App {
    pub cfg: Config,
    pub theme: usize,
    pub engine: Option<Engine>,
    pub local: Vec<Track>,
    pub remote: Vec<Track>,
    pub source: usize,
    pub sel: usize,
    pub scroll: usize,
    pub pl: Vec<Track>,
    pub pl_idx: usize,
    pub now: Option<Track>,
    pub stream: Option<Stream>,
    /// An online track prepared in the background so enter starts fast.
    prefetch: Option<(String, Stream)>,
    /// Track waiting out the selection debounce before prefetching.
    prefetch_want: Option<(String, f64)>,
    prefetch_hold: f64,
    pub buffering: bool,
    pub pos: f64,
    pub dur: f64,
    pub playing: bool,
    pub query: String,
    pub mode: Mode,
    pub lib_open: bool,
    pub bars: Vec<f32>,
    pub status: String,
    pub searching: bool,
    pub saving: usize,
    pub have_ytdlp: bool,
    pub have_ffmpeg: bool,
    pub set_tab: usize,
    pub theme_state: ListState,
    pub dir_input: String,
    pub dir_msg: Option<(String, bool)>,
    pub pick_i: usize,
    pub devices: Vec<String>,
    pub dev_state: ListState,
    pub needs_clear: bool,
    pub playlists: Vec<Playlist>,
    pub open_list: Option<usize>,
    pub pick_state: ListState,
    pub pending: Option<Track>,
    pub name_input: String,
    pub name_err: Option<String>,
    pub rename: Option<usize>,
    confirm_del: Option<usize>,
    pub hit: Hit,
    pub quit: bool,
    search_gen: u64,
    cache: HashMap<SearchKey, Vec<Track>>,
    tx: Sender<Msg>,
    rx: Receiver<Msg>,
    pub room: Option<RoomHandle>,
    room_key: String,
    room_timer: f64,
    pub share_code: String,
    pub share_title: String,
    pub share_path: Option<std::path::PathBuf>,
    pub share_copied: bool,
    pub join_input: String,
    pub join_err: Option<String>,
    pub room_ads: Vec<beacon::RoomAd>,
    pub room_pick: usize,
    room_punch: Option<punch::Reach>,
    beacon: Option<beacon::Announcer>,
    beacon_srv: String,
}

/// Edits a text box. Handles Backspace even when the terminal sends it as Ctrl+H.
fn edit_text(s: &mut String, k: &KeyEvent) -> bool {
    let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
    match k.code {
        KeyCode::Backspace => {
            s.pop();
            true
        }
        KeyCode::Char('h') if ctrl => {
            s.pop();
            true
        }
        KeyCode::Char('u') if ctrl => {
            s.clear();
            true
        }
        KeyCode::Char('w') if ctrl => {
            while s.ends_with(' ') {
                s.pop();
            }
            while !s.is_empty() && !s.ends_with(' ') {
                s.pop();
            }
            true
        }
        KeyCode::Char(c) if !ctrl => {
            s.push(c);
            true
        }
        _ => false,
    }
}

fn inside(r: Rect, x: u16, y: u16) -> bool {
    x >= r.x && x < r.x + r.width && y >= r.y && y < r.y + r.height
}

fn frac(r: Rect, x: u16) -> f32 {
    if r.width == 0 {
        return 0.0;
    }
    ((x.saturating_sub(r.x)) as f32 / r.width as f32).clamp(0.0, 1.0)
}

/// Do two track references point at the same song? falls back to tags so a
/// guest can follow a shared local file they already have somewhere else.
fn same_track(a: &Track, b: &Track) -> bool {
    (a.url.is_some() && a.url == b.url)
        || (a.path.is_some() && a.path == b.path)
        || (a.title.to_lowercase() == b.title.to_lowercase()
            && a.artist.to_lowercase() == b.artist.to_lowercase())
}

impl App {
    pub fn new() -> App {
        let cfg = Config::load();
        let theme = themes::index_of(&cfg.theme);
        let (engine, engine_err) = match Engine::new(&cfg.device) {
            Ok(e) => (Some(e), None),
            Err(e) => (None, Some(e)),
        };
        let (tx, rx) = channel();
        stream::clear_cache();
        let mut app = App {
            dir_input: cfg.dir.clone(),
            cfg,
            theme,
            engine,
            local: Vec::new(),
            remote: Vec::new(),
            source: 0,
            sel: 0,
            scroll: 0,
            pl: Vec::new(),
            pl_idx: 0,
            now: None,
            stream: None,
            prefetch: None,
            prefetch_want: None,
            prefetch_hold: 0.0,
            buffering: false,
            pos: 0.0,
            dur: 0.0,
            playing: false,
            query: String::new(),
            mode: Mode::Normal,
            lib_open: true,
            bars: vec![0.0; BANDS],
            status: String::new(),
            searching: false,
            saving: 0,
            have_ytdlp: library::has_ytdlp(),
            have_ffmpeg: library::has_ffmpeg(),
            set_tab: 0,
            theme_state: ListState::default(),
            dir_msg: None,
            pick_i: 0,
            devices: Vec::new(),
            dev_state: ListState::default(),
            needs_clear: false,
            playlists: playlists::load(),
            open_list: None,
            pick_state: ListState::default(),
            pending: None,
            name_input: String::new(),
            name_err: None,
            rename: None,
            confirm_del: None,
            hit: Hit::default(),
            quit: false,
            search_gen: 0,
            cache: HashMap::new(),
            tx,
            rx,
            room: None,
            room_key: String::new(),
            room_timer: 0.0,
            share_code: String::new(),
            share_title: String::new(),
            share_path: None,
            share_copied: false,
            join_input: String::new(),
            join_err: None,
            room_ads: Vec::new(),
            room_pick: 0,
            room_punch: None,
            beacon: None,
            beacon_srv: String::new(),
        };
        app.apply_audio();
        app.rescan();
        // LAN discovery: hear other hosts' rooms on this network.
        beacon::listen_once();
        if let Some(e) = engine_err {
            app.status = format!("no audio output: {e}");
        }
        app
    }

    pub fn save(&mut self) {
        self.cfg.theme = THEMES[self.theme].name.to_string();
        self.cfg.save();
    }

    /// Called once on exit: stop streams and remove temporary audio.
    pub fn shutdown(&mut self) {
        if let Some(r) = self.room.take() {
            r.leave();
        }
        self.end_beacon();
        if let Some(reach) = self.room_punch.take() {
            thread::spawn(move || punch::forget(&reach));
        }
        self.drop_prefetch();
        self.drop_stream();
        stream::clear_cache();
        self.save();
    }

    pub fn display_name(&self) -> String {
        share::sanitize(self.cfg.name.trim())
            .chars()
            .take(24)
            .collect()
    }

    /// True while this client is a listener following a host's playback.
    pub fn following(&self) -> bool {
        self.room.as_ref().map(|r| !r.host).unwrap_or(false)
    }

    // ---------- library ----------

    pub fn rescan(&mut self) {
        let dir = expand(&self.cfg.dir);
        let tx = self.tx.clone();
        self.status = format!("scanning {} ...", self.cfg.dir);
        thread::spawn(move || {
            let tracks = library::scan(&dir);
            let _ = tx.send(Msg::Scanned(tracks));
        });
    }

    pub fn list(&self) -> Vec<&Track> {
        if self.source == 0 {
            let q = self.query.to_lowercase();
            self.local
                .iter()
                .filter(|t| q.is_empty() || t.key.contains(&q))
                .collect()
        } else if self.source == PLAYLISTS {
            match self.open_list.and_then(|i| self.playlists.get(i)) {
                Some(p) => {
                    let q = self.query.to_lowercase();
                    p.tracks
                        .iter()
                        .filter(|t| q.is_empty() || t.key.contains(&q))
                        .collect()
                }
                None => Vec::new(),
            }
        } else {
            self.remote.iter().collect()
        }
    }

    /// Number of rows in the current view (playlists, or tracks).
    pub fn rows(&self) -> usize {
        if self.source == PLAYLISTS && self.open_list.is_none() {
            self.playlists.len()
        } else {
            self.list().len()
        }
    }

    fn fix_sel(&mut self) {
        let n = self.rows();
        if n == 0 {
            self.sel = 0;
        } else if self.sel >= n {
            self.sel = n - 1;
        }
    }

    fn move_sel(&mut self, d: isize) {
        let n = self.rows();
        if n == 0 {
            self.sel = 0;
            self.schedule_prefetch();
            return;
        }
        self.sel = (self.sel as isize + d).clamp(0, n as isize - 1) as usize;
        self.schedule_prefetch();
    }

    fn set_source(&mut self, i: usize) {
        if i >= Provider::ALL.len() {
            return;
        }
        self.source = i;
        self.sel = 0;
        self.scroll = 0;
        let p = Provider::ALL[i];
        if p == Provider::Playlists {
            self.open_list = None;
        }
        self.status = match p {
            Provider::Local => String::new(),
            Provider::Playlists => "enter opens a playlist. select a track and press a to add it".into(),
            _ if !self.have_ytdlp => "yt-dlp not found. Install it to use online sources.".into(),
            Provider::Bandcamp => "bandcamp: press / and paste a track or album url".into(),
            _ => "press / to search, then enter".into(),
        };
        self.schedule_prefetch();
    }

    fn start_search(&mut self) {
        let q = self.query.trim().to_string();
        if q.is_empty() {
            return;
        }
        if !self.have_ytdlp {
            self.status = "yt-dlp not found. Install it to use online sources.".into();
            return;
        }
        let provider = Provider::ALL[self.source];
        let is_url = q.starts_with("http://") || q.starts_with("https://");
        if provider == Provider::Bandcamp && !is_url {
            self.status = "bandcamp needs a track or album url".into();
            return;
        }

        let key: SearchKey = (self.source, q.to_lowercase());
        if let Some(hit) = self.cache.get(&key) {
            self.remote = hit.clone();
            self.sel = 0;
            self.scroll = 0;
            self.status = format!("{} results (cached). enter plays, d saves", self.remote.len());
            self.schedule_prefetch();
            return;
        }

        // a new search replaces any search still running
        self.drop_prefetch();
        self.search_gen += 1;
        let seq = self.search_gen;
        self.searching = true;
        self.status = format!("searching {} ...", provider.name());
        let tx = self.tx.clone();
        thread::spawn(move || {
            let tx2 = tx.clone();
            let res = library::search_remote(provider, &q, &move |a, m| {
                let _ = tx2.send(Msg::Note(seq, format!("service busy, retrying ({a}/{m})")));
            });
            let _ = tx.send(Msg::Found { seq, key, res });
        });
    }

    fn save_track(&mut self, t: Track) {
        let Some(url) = t.url.clone() else { return };
        let dir = expand(&self.cfg.dir);
        let tx = self.tx.clone();
        self.saving += 1;
        self.status = format!("saving: {} ...", t.title);
        thread::spawn(move || {
            let res = library::download(&url, &dir, &|_, _| {}).map(|p| library::track_from_path(&p));
            let _ = tx.send(Msg::Saved(res));
        });
    }

    // ---------- playback ----------

    pub fn apply_audio(&mut self) {
        let v = if self.cfg.muted { 0.0 } else { self.cfg.vol };
        let (r, w, s) = (self.cfg.reverb, self.cfg.wet, self.cfg.speed);
        if let Some(e) = self.engine.as_mut() {
            e.set_volume(v);
            e.set_speed(s);
            e.set_reverb(r, w);
        }
    }

    /// Stops the background prefetch, if any, and removes its cache file.
    fn drop_prefetch(&mut self) {
        self.prefetch_want = None;
        self.prefetch_hold = 0.0;
        if let Some((_, s)) = self.prefetch.take() {
            s.kill.store(true, Ordering::Relaxed);
            let _ = std::fs::remove_file(&s.path);
        }
    }

    /// Prepares the selected online track in the background, so pressing
    /// enter does not have to wait for yt-dlp and the first seconds of audio.
    fn schedule_prefetch(&mut self) {
        self.prefetch_want = None;
        if self.source == 0 || self.source == PLAYLISTS || self.searching {
            self.drop_prefetch();
            return;
        }
        if !self.have_ytdlp || !self.have_ffmpeg || self.engine.is_none() {
            self.drop_prefetch();
            return;
        }
        let picked = self.list().get(self.sel).map(|t| (t.url.clone(), t.secs));
        let Some((Some(url), secs)) = picked else {
            self.drop_prefetch();
            return;
        };
        // nothing to do when this url is already playing
        if self.stream.is_some() && self.now.as_ref().and_then(|t| t.url.as_deref()) == Some(url.as_str()) {
            self.drop_prefetch();
            return;
        }
        // nothing to do when this url is already prepared
        if let Some((u, _)) = &self.prefetch {
            if *u == url {
                return;
            }
        }
        self.drop_prefetch();
        self.prefetch_want = Some((url, secs));
        self.prefetch_hold = 0.4;
    }

    /// Hands back the prefetched stream for `url` if it is still healthy.
    fn take_prefetch(&mut self, url: &str) -> Option<Stream> {
        let (u, s) = self.prefetch.take()?;
        if u == url {
            let failed = s.written.load(Ordering::Relaxed) == 0
                && s.error.lock().map(|e| e.is_some()).unwrap_or(false);
            if !failed {
                return Some(s);
            }
        }
        s.kill.store(true, Ordering::Relaxed);
        let _ = std::fs::remove_file(&s.path);
        None
    }

    fn drop_stream(&mut self) {
        if let Some(s) = self.stream.take() {
            s.kill.store(true, Ordering::Relaxed);
            let _ = std::fs::remove_file(&s.path);
        }
        self.buffering = false;
    }

    fn play_any(&mut self, t: Track) {
        if t.path.is_some() {
            self.play_track(t);
        } else {
            self.play_remote(t);
        }
    }

    fn play_track(&mut self, t: Track) {
        let Some(path) = t.path.clone() else {
            return;
        };
        self.drop_prefetch();
        self.drop_stream();
        let Some(engine) = self.engine.as_mut() else {
            self.status = "no audio output device".into();
            return;
        };
        match engine.play(&path) {
            Ok(()) => {
                self.pos = 0.0;
                self.dur = t.secs;
                self.now = Some(t);
                self.playing = true;
                self.status.clear();
            }
            Err(e) => self.status = format!("cannot play: {e}"),
        }
    }

    fn play_remote(&mut self, t: Track) {
        let Some(url) = t.url.clone() else {
            return;
        };
        if !self.have_ytdlp || !self.have_ffmpeg {
            self.status = "streaming needs yt-dlp and ffmpeg installed".into();
            return;
        }
        let rate = match self.engine.as_ref() {
            Some(e) => e.out_rate(),
            None => {
                self.status = "no audio output device".into();
                return;
            }
        };
        self.drop_stream();
        let s = match self.take_prefetch(&url) {
            Some(s) => s,
            None => match Stream::start(&url, rate, t.secs) {
                Ok(s) => s,
                Err(e) => {
                    self.status = e;
                    return;
                }
            },
        };
        let res = match self.engine.as_mut() {
            Some(e) => e.play_stream(&s),
            None => Err("no audio output device".to_string()),
        };
        match res {
            Ok(()) => {
                self.pos = 0.0;
                self.dur = t.secs;
                self.now = Some(t);
                self.playing = true;
                self.stream = Some(s);
                self.buffering = true;
                self.status = "buffering ...".into();
            }
            Err(e) => {
                s.kill.store(true, Ordering::Relaxed);
                self.status = format!("cannot play: {e}");
            }
        }
    }

    fn toggle_play(&mut self) {
        if self.now.is_none() {
            self.activate(true);
            return;
        }
        if self.following() {
            self.status = "you are following the host. press L to leave the room".into();
            return;
        }
        self.set_playing(!self.playing);
    }

    fn set_playing(&mut self, on: bool) {
        if self.playing == on {
            return;
        }
        self.playing = on;
        if let Some(e) = self.engine.as_mut() {
            if on {
                e.resume();
            } else {
                e.pause();
            }
        }
    }

    fn next(&mut self, d: isize) {
        if self.following() {
            self.status = "in a shared room only the host can change the track".into();
            return;
        }
        if self.pl.is_empty() {
            return;
        }
        let n = self.pl.len() as isize;
        let idx = (self.pl_idx as isize + d).rem_euclid(n) as usize;
        self.pl_idx = idx;
        let t = self.pl[idx].clone();
        self.play_any(t);
    }

    fn prev(&mut self) {
        if self.following() {
            self.status = "in a shared room only the host can seek".into();
            return;
        }
        if self.pos > 3.0 {
            self.do_seek(0.0);
        } else {
            self.next(-1);
        }
    }

    pub fn seek_to(&mut self, t: f64) {
        if self.following() {
            self.status = "in a shared room only the host can seek".into();
            return;
        }
        self.do_seek(t);
    }

    fn do_seek(&mut self, t: f64) {
        if self.now.is_none() {
            return;
        }
        let path = self.now.as_ref().and_then(|n| n.path.clone());
        let t = if self.dur > 0.0 {
            t.clamp(0.0, (self.dur - 0.05).max(0.0))
        } else {
            t.max(0.0)
        };
        let ok = match self.engine.as_mut() {
            Some(e) => {
                if e.is_stream() {
                    e.seek_stream(t)
                } else {
                    match &path {
                        Some(p) => e.seek(p, t),
                        None => false,
                    }
                }
            }
            None => false,
        };
        if ok {
            self.pos = t;
        } else {
            self.status = "seek failed for this file".into();
        }
    }

    fn seek_by(&mut self, d: f64) {
        self.seek_to(self.pos + d);
    }

    fn set_vol(&mut self, v: f32) {
        self.cfg.vol = v.clamp(0.0, 1.0);
        self.cfg.muted = self.cfg.vol <= 0.0;
        self.apply_audio();
    }

    fn toggle_mute(&mut self) {
        self.cfg.muted = !self.cfg.muted;
        if !self.cfg.muted && self.cfg.vol <= 0.0 {
            self.cfg.vol = 0.5;
        }
        self.apply_audio();
    }

    fn set_speed(&mut self, s: f32) {
        self.cfg.speed = ((s.clamp(0.5, 2.0)) * 100.0).round() / 100.0;
        self.apply_audio();
    }

    fn set_wet(&mut self, w: u32) {
        let w = ((w.min(100) + 2) / 5) * 5;
        self.cfg.wet = w;
        if w == 0 {
            self.cfg.reverb = 0;
        } else if self.cfg.reverb == 0 {
            self.cfg.reverb = 1;
        }
        self.apply_audio();
    }

    fn set_reverb_preset(&mut self, i: usize) {
        let i = i.min(REVERBS.len() - 1);
        self.cfg.reverb = i;
        self.cfg.wet = REVERBS[i].1;
        self.apply_audio();
    }

    fn activate(&mut self, play: bool) {
        if self.following() {
            self.status = "in a shared room the host picks the music. press L to leave".into();
            return;
        }
        if self.source == PLAYLISTS && self.open_list.is_none() {
            if play && self.sel < self.playlists.len() {
                self.open_list = Some(self.sel);
                self.sel = 0;
                self.scroll = 0;
                self.query.clear();
            }
            return;
        }
        let (tracks, chosen) = {
            let l = self.list();
            (
                l.iter().map(|t| (*t).clone()).collect::<Vec<Track>>(),
                l.get(self.sel).map(|t| (*t).clone()),
            )
        };
        let Some(t) = chosen else { return };
        if play {
            self.pl = tracks;
            self.pl_idx = self.sel;
            self.play_any(t);
        } else if self.source != 0 {
            self.save_track(t);
        }
    }

    // ---------- per-frame ----------

    /// Returns true when something on screen changed.
    pub fn tick(&mut self, dt: f64) -> bool {
        let mut changed = false;

        while let Ok(m) = self.rx.try_recv() {
            changed = true;
            match m {
                Msg::Scanned(t) => {
                    self.local = t;
                    self.status = format!("{} local tracks in {}", self.local.len(), self.cfg.dir);
                    self.fix_sel();
                }
                Msg::Note(seq, text) => {
                    if seq == self.search_gen {
                        self.status = text;
                    }
                }
                Msg::Found { seq, key, res } => {
                    if seq != self.search_gen {
                        continue;
                    }
                    self.searching = false;
                    match res {
                        Ok(t) => {
                            self.status = format!("{} results. enter plays, d saves", t.len());
                            if self.cache.len() > 32 {
                                self.cache.clear();
                            }
                            self.cache.insert(key, t.clone());
                            self.remote = t;
                            self.sel = 0;
                            self.scroll = 0;
                            self.schedule_prefetch();
                        }
                        Err(e) => self.status = e,
                    }
                }
                Msg::Saved(res) => {
                    self.saving = self.saving.saturating_sub(1);
                    match res {
                        Ok(t) => {
                            self.status = format!("saved: {}", t.title);
                            if !self.local.iter().any(|x| x.path == t.path) {
                                self.local.push(t);
                            }
                        }
                        Err(e) => self.status = e,
                    }
                }
                Msg::Together(e) => self.on_room_event(e),
                Msg::Reach(reach) => self.on_reach(reach),
            }
        }

        // start a queued prefetch once the selection has settled
        if let Some((url, secs)) = self.prefetch_want.take() {
            self.prefetch_hold -= dt;
            if self.prefetch_hold > 0.0 {
                self.prefetch_want = Some((url, secs));
            } else if let Some(rate) = self.engine.as_ref().map(|e| e.out_rate()) {
                if let Ok(s) = Stream::start(&url, rate, secs) {
                    self.prefetch = Some((url, s));
                }
            }
        }

        if self.playing && self.now.is_some() {
            changed = true;

            // position: exact for streams, clock for files
            let sp = self.engine.as_ref().and_then(|e| e.stream_pos());
            match sp {
                Some(p) => self.pos = p,
                None => self.pos += dt * self.cfg.speed as f64,
            }

            // stream health
            let mut err: Option<String> = None;
            let mut note = String::new();
            let mut done_dur: Option<f64> = None;
            let mut buffering = false;
            if let Some(s) = &self.stream {
                let w = s.written.load(Ordering::Relaxed);
                let c = s.consumed.load(Ordering::Relaxed) * 2;
                let ahead = w.saturating_sub(c);
                let done = s.done.load(Ordering::Acquire);
                buffering = !done && ahead < (s.rate as u64 * 4) / 5;
                if done && self.dur <= 0.0 {
                    done_dur = Some(w as f64 / (4.0 * s.rate as f64));
                }
                if let Ok(mut e) = s.error.lock() {
                    err = e.take();
                }
                if let Ok(n) = s.note.lock() {
                    note = n.clone();
                }
            }
            self.buffering = buffering;
            if let Some(d) = done_dur {
                self.dur = d;
            }
            if let Some(e) = err {
                self.status = e;
                self.playing = false;
                if let Some(en) = self.engine.as_mut() {
                    en.pause();
                }
            } else if buffering {
                self.status = if note.is_empty() { "buffering ...".to_string() } else { note };
            } else if self.status.starts_with("buffering") || self.status.starts_with("service busy") {
                self.status.clear();
            }

            let done = self.engine.as_ref().map(|e| e.finished()).unwrap_or(false);
            if done {
                if self.following() {
                    // the host sends the next track; do not run ahead of it
                    self.playing = false;
                    if let Some(en) = self.engine.as_mut() {
                        en.pause();
                    }
                } else {
                    self.next(1);
                }
            }
        }

        let raw = match (&self.engine, self.playing) {
            (Some(e), true) => e.spectrum(BANDS),
            _ => vec![0.0; BANDS],
        };
        let decay = if self.playing { 0.45 + 0.5 * (self.cfg.wet as f32 / 100.0) } else { 0.3 };
        for (i, b) in self.bars.iter_mut().enumerate() {
            let v = raw.get(i).copied().unwrap_or(0.0);
            *b = v.max(*b * decay);
            if *b < 0.01 {
                *b = 0.0;
            } else {
                changed = true;
            }
        }

        // while the join window is open, keep the LAN room list current
        if self.mode == Mode::Join {
            let ads = beacon::rooms();
            if ads != self.room_ads {
                self.room_ads = ads;
                if self.room_pick >= self.room_ads.len() {
                    self.room_pick = 0;
                }
                changed = true;
            }
        }

        self.room_tick(dt);

        changed
    }

    // ---------- keyboard ----------

    pub fn on_key(&mut self, k: KeyEvent) {
        if k.modifiers.contains(KeyModifiers::CONTROL) && k.code == KeyCode::Char('c') {
            self.quit = true;
            return;
        }
        match self.mode {
            Mode::Normal => self.key_normal(k),
            Mode::Search => self.key_search(k),
            Mode::Settings => self.key_settings(k),
            Mode::Pick => self.key_pick(k),
            Mode::Name => self.key_name(k),
            Mode::Share => self.key_share(k),
            Mode::Join => self.key_join(k),
            Mode::Room => self.key_room(k),
        }
    }

    fn key_normal(&mut self, k: KeyEvent) {
        if k.code != KeyCode::Char('x') {
            self.confirm_del = None;
        }
        match k.code {
            KeyCode::Char('q') => self.quit = true,
            KeyCode::Char('/') => self.mode = Mode::Search,
            KeyCode::Char('s') | KeyCode::Char(',') => self.open_settings(),
            KeyCode::Tab => self.lib_open = !self.lib_open,
            KeyCode::Char(' ') => self.toggle_play(),
            KeyCode::Char('h') if k.modifiers.contains(KeyModifiers::CONTROL) => self.close_list(),
            KeyCode::Left | KeyCode::Char('h') => self.seek_by(-5.0),
            KeyCode::Right | KeyCode::Char('l') => self.seek_by(5.0),
            KeyCode::Up | KeyCode::Char('k') => self.move_sel(-1),
            KeyCode::Down | KeyCode::Char('j') => self.move_sel(1),
            KeyCode::PageUp => self.move_sel(-10),
            KeyCode::PageDown => self.move_sel(10),
            KeyCode::Enter => self.activate(true),
            KeyCode::Char('d') => self.activate(false),
            KeyCode::Char('-') => self.set_vol(self.cfg.vol - 0.05),
            KeyCode::Char('=') | KeyCode::Char('+') => self.set_vol(self.cfg.vol + 0.05),
            KeyCode::Char('m') => self.toggle_mute(),
            KeyCode::Char('[') => self.set_speed(self.cfg.speed - 0.1),
            KeyCode::Char(']') => self.set_speed(self.cfg.speed + 0.1),
            KeyCode::Char('r') => self.set_reverb_preset((self.cfg.reverb + 1) % REVERBS.len()),
            KeyCode::Char('<') => self.set_wet(self.cfg.wet.saturating_sub(5)),
            KeyCode::Char('>') => self.set_wet(self.cfg.wet + 5),
            KeyCode::Char('n') => self.next(1),
            KeyCode::Char('p') => self.prev(),
            KeyCode::Char('a') => self.begin_add(false),
            KeyCode::Char('A') => self.begin_add(true),
            KeyCode::Char('H') => {
                if self.room.is_some() {
                    self.mode = Mode::Room;
                } else {
                    self.start_hosting();
                }
            }
            KeyCode::Char('U') => {
                if self.room.is_some() {
                    self.status = "you are already in a room. press L to leave first".into();
                } else {
                    self.join_input.clear();
                    self.join_err = None;
                    self.refresh_ads();
                    self.mode = Mode::Join;
                }
            }
            KeyCode::Char('S') => self.share_current(),
            KeyCode::Char('L') => self.leave_room(),
            KeyCode::Char('c') => {
                if self.source == PLAYLISTS && self.open_list.is_none() {
                    self.pending = None;
                    self.open_name(None, String::new());
                }
            }
            KeyCode::Char('R') => {
                if self.source == PLAYLISTS && self.open_list.is_none() && self.sel < self.playlists.len() {
                    let n = self.playlists[self.sel].name.clone();
                    self.open_name(Some(self.sel), n);
                }
            }
            KeyCode::Char('x') => self.remove_selected(),
            KeyCode::Char('J') => self.move_item(1),
            KeyCode::Char('K') => self.move_item(-1),
            KeyCode::Backspace | KeyCode::Esc => self.close_list(),
            KeyCode::Char(c @ '1'..='5') => self.set_source(c as usize - '1' as usize),
            _ => {}
        }
    }

    fn key_search(&mut self, k: KeyEvent) {
        match k.code {
            KeyCode::Esc => {
                self.query.clear();
                self.mode = Mode::Normal;
                self.sel = 0;
                self.scroll = 0;
            }
            KeyCode::Enter => {
                self.mode = Mode::Normal;
                if self.source != 0 && self.source != PLAYLISTS {
                    self.start_search();
                }
            }
            KeyCode::Up => self.move_sel(-1),
            KeyCode::Down => self.move_sel(1),
            _ => {
                if edit_text(&mut self.query, &k) {
                    self.sel = 0;
                    self.scroll = 0;
                }
            }
        }
    }

    // ---------- playlists ----------

    fn save_lists(&self) {
        playlists::save(&self.playlists);
    }

    fn open_name(&mut self, rename: Option<usize>, text: String) {
        self.rename = rename;
        self.name_input = text;
        self.name_err = None;
        self.mode = Mode::Name;
    }

    /// `a` adds the selected track, `A` adds the track that is playing.
    fn begin_add(&mut self, use_now: bool) {
        let t: Option<Track> = if use_now {
            self.now.clone()
        } else if self.source == PLAYLISTS && self.open_list.is_none() {
            None
        } else {
            let l = self.list();
            let r = l.get(self.sel).map(|t| (*t).clone());
            r
        };
        let Some(t) = t else {
            self.status = "nothing to add: select a track first".into();
            return;
        };
        if t.path.is_none() && t.url.is_none() {
            return;
        }
        self.pending = Some(t);
        if self.playlists.is_empty() {
            self.open_name(None, String::new());
        } else {
            self.pick_state.select(Some(0));
            self.mode = Mode::Pick;
        }
    }

    fn add_pending(&mut self, idx: usize) {
        let Some(t) = self.pending.take() else { return };
        let Some(pl) = self.playlists.get_mut(idx) else { return };
        if pl.contains(&t) {
            self.status = format!("already in {}", pl.name);
            return;
        }
        let name = pl.name.clone();
        pl.tracks.push(t);
        self.save_lists();
        self.status = format!("added to {name}");
    }

    fn key_pick(&mut self, k: KeyEvent) {
        let n = self.playlists.len() + 1;
        let cur = self.pick_state.selected().unwrap_or(0);
        match k.code {
            KeyCode::Esc => {
                self.pending = None;
                self.mode = Mode::Normal;
            }
            KeyCode::Up | KeyCode::Char('k') => self.pick_state.select(Some((cur + n - 1) % n)),
            KeyCode::Down | KeyCode::Char('j') => self.pick_state.select(Some((cur + 1) % n)),
            KeyCode::Enter => {
                if cur == 0 {
                    self.open_name(None, String::new());
                } else {
                    self.add_pending(cur - 1);
                    self.mode = Mode::Normal;
                }
            }
            _ => {}
        }
    }

    fn key_name(&mut self, k: KeyEvent) {
        match k.code {
            KeyCode::Esc => {
                self.pending = None;
                self.mode = Mode::Normal;
            }
            KeyCode::Enter => self.commit_name(),
            _ => {
                if edit_text(&mut self.name_input, &k) {
                    self.name_err = None;
                    if self.name_input.chars().count() > 40 {
                        self.name_input.pop();
                    }
                }
            }
        }
    }

    fn commit_name(&mut self) {
        let name = self.name_input.trim().to_string();
        if name.is_empty() {
            self.name_err = Some("Type a name first.".into());
            return;
        }
        let taken = self
            .playlists
            .iter()
            .enumerate()
            .any(|(i, p)| p.name.to_lowercase() == name.to_lowercase() && Some(i) != self.rename);
        if taken {
            self.name_err = Some("A playlist with that name already exists.".into());
            return;
        }
        match self.rename {
            Some(i) => {
                if let Some(p) = self.playlists.get_mut(i) {
                    p.name = name.clone();
                }
                self.status = format!("renamed to {name}");
            }
            None => {
                self.playlists.push(Playlist { name: name.clone(), tracks: Vec::new() });
                let idx = self.playlists.len() - 1;
                self.status = format!("created {name}");
                if self.pending.is_some() {
                    self.add_pending(idx);
                }
            }
        }
        self.save_lists();
        self.mode = Mode::Normal;
    }

    /// `x`: remove the selected track, or delete the selected playlist (press twice).
    fn remove_selected(&mut self) {
        if self.source != PLAYLISTS {
            return;
        }
        match self.open_list {
            Some(i) => {
                if !self.query.is_empty() {
                    self.status = "clear the search first (esc), then remove".into();
                    return;
                }
                let n = self.playlists.get(i).map(|p| p.tracks.len()).unwrap_or(0);
                if self.sel < n {
                    let t = self.playlists[i].tracks.remove(self.sel);
                    self.status = format!("removed: {}", t.title);
                    self.save_lists();
                    self.fix_sel();
                }
            }
            None => {
                if self.sel >= self.playlists.len() {
                    return;
                }
                if self.confirm_del == Some(self.sel) {
                    let p = self.playlists.remove(self.sel);
                    self.confirm_del = None;
                    self.status = format!("deleted playlist: {}", p.name);
                    self.save_lists();
                    self.fix_sel();
                } else {
                    self.confirm_del = Some(self.sel);
                    self.status = format!("press x again to delete \"{}\"", self.playlists[self.sel].name);
                }
            }
        }
    }

    /// `J` / `K`: move the selected track (or playlist) down / up.
    fn move_item(&mut self, d: isize) {
        if self.source != PLAYLISTS || !self.query.is_empty() {
            return;
        }
        let a = self.sel;
        let b = a as isize + d;
        match self.open_list {
            Some(i) => {
                let n = self.playlists.get(i).map(|p| p.tracks.len()).unwrap_or(0);
                if b < 0 || b >= n as isize || a >= n {
                    return;
                }
                self.playlists[i].tracks.swap(a, b as usize);
            }
            None => {
                let n = self.playlists.len();
                if b < 0 || b >= n as isize || a >= n {
                    return;
                }
                self.playlists.swap(a, b as usize);
            }
        }
        self.sel = b as usize;
        self.save_lists();
    }

    fn close_list(&mut self) {
        if self.source == PLAYLISTS {
            if let Some(i) = self.open_list.take() {
                self.sel = i;
                self.scroll = 0;
                self.query.clear();
            }
        }
    }

    // ---------- listen together ----------

    /// What this machine would report as the room's playback state.
    fn room_state_now(&self) -> RoomState {
        let queue = self.pl.iter().map(SharedTrack::from_track).collect();
        let index = if self.pl.is_empty() { 0 } else { self.pl_idx.min(self.pl.len() - 1) };
        RoomState {
            label: self
                .room
                .as_ref()
                .map(|r| r.state.label.clone())
                .unwrap_or_else(|| "shared session".into()),
            queue,
            index,
            pos: self.pos,
            playing: self.playing,
            speed: self.cfg.speed,
        }
    }

    /// `H`: host the current session (queue, or the open playlist) on the sync server.
    fn start_hosting(&mut self) {
        if self.room.is_some() {
            self.mode = Mode::Room;
            return;
        }
        if self.source == PLAYLISTS && self.open_list.is_none() {
            let Some(pl) = self.playlists.get(self.sel).cloned() else {
                self.status = "select a playlist to share, or open one first".into();
                return;
            };
            if pl.tracks.is_empty() {
                self.status = "that playlist is empty".into();
                return;
            }
            self.pl = pl.tracks.clone();
            self.pl_idx = 0;
        } else if self.pl.is_empty() {
            let list = self.list().into_iter().cloned().collect::<Vec<Track>>();
            if list.is_empty() {
                self.status = "nothing to share yet: play something first".into();
                return;
            }
            self.pl = list;
            self.pl_idx = self.sel.min(self.pl.len() - 1);
        }
        let label = if self.source == PLAYLISTS {
            self.open_list
                .and_then(|i| self.playlists.get(i))
                .map(|p| p.name.clone())
                .unwrap_or_else(|| "shared session".into())
        } else {
            self.now.as_ref().map(|n| n.title.clone()).unwrap_or_else(|| "shared session".into())
        };
        let st = RoomState {
            label: share::sanitize(&label).chars().take(40).collect(),
            queue: Vec::new(),
            index: self.pl_idx,
            pos: self.pos,
            playing: self.playing,
            speed: self.cfg.speed,
        };
        let st = RoomState {
            queue: self.pl.iter().map(SharedTrack::from_track).collect(),
            ..st
        };

        let me = self.display_name();
        let (etx, erx) = channel::<Event>();
        let bridge = self.tx.clone();
        let bridge2 = bridge.clone();
        thread::spawn(move || {
            while let Ok(e) = erx.recv() {
                if bridge2.send(Msg::Together(e)).is_err() {
                    break;
                }
            }
        });
        // Where the room's relay lives: a manual one if set (env or config),
        // otherwise this app hosts it itself and tries to open the router for it.
        let relay = self.relay_override();
        let local = match &relay {
            Some(s) => s.clone(),
            None => match self.embed_relay(&bridge) {
                Ok(a) => a,
                Err(e) => {
                    self.status = e;
                    return;
                }
            },
        };
        match together::create(&local, &me, st.clone(), etx) {
            Ok(h) => {
                self.room = Some(h);
                self.room_key.clear();
                self.room_timer = 0.0;
                self.status = if relay.is_some() {
                    format!("creating room on {local} ...")
                } else {
                    "room opening here - invite appears when it is live".into()
                };
            }
            Err(e) => {
                self.status = e;
                return;
            }
        }
        self.refresh_beacon();
        self.mode = Mode::Room;
    }

    /// A manually configured relay, if the user (or env) pinned one.
    fn relay_override(&self) -> Option<String> {
        if let Some(s) = crate::config::env_server() {
            return Some(s);
        }
        match self.cfg.server.trim() {
            "" => None,
            s => Some(s.to_string()),
        }
    }

    /// Hosts the room inside this app: binds the own relay on all interfaces
    /// and starts the automatic router port-opening in the background.
    fn embed_relay(&mut self, bridge: &Sender<Msg>) -> Result<String, String> {
        let listener = TcpListener::bind("0.0.0.0:0").map_err(|e| format!("cannot open a room here: {e}"))?;
        let addr = listener
            .local_addr()
            .map_err(|e| format!("cannot open a room here: {e}"))?;
        let _ = thread::Builder::new().name("room-relay".into()).spawn(move || {
            let _ = crate::server::serve(listener);
        });
        let tx = bridge.clone();
        thread::Builder::new()
            .name("room-punch".into())
            .spawn(move || {
                let lan = punch::local_ip();
                let _ = tx.send(Msg::Reach(punch::open_port(addr.port(), lan)));
            })
            .ok();
        Ok(self_addr(addr.port()))
    }

    /// Applies the router result to the room and its announce beacon.
    fn on_reach(&mut self, reach: punch::Reach) {
        let servers = reach.servers.join("|");
        self.room_punch = Some(reach);
        if let Some(h) = self.room.as_mut() {
            if h.host && !h.pending && !servers.is_empty() {
                h.server = servers;
            }
        }
        if let Some(r) = &self.room_punch {
            self.status = if r.servers.len() > 1 {
                "room is open to the internet - paste the invite to friends anywhere".into()
            } else if r.servers.is_empty() {
                format!("room is on this network only ({})", r.note)
            } else {
                format!("room ready; {}", r.note)
            };
        }
        self.refresh_beacon();
    }

    /// Keeps the LAN beacon pointing at the live room (code + addresses).
    fn refresh_beacon(&mut self) {
        let want = self
            .room
            .as_ref()
            .filter(|r| r.host && !r.pending && !r.code.is_empty())
            .map(|r| (r.code.clone(), self.display_name(), r.server.clone()));
        match want {
            Some((code, name, srv)) => {
                if self.beacon.is_none() || self.beacon_srv != srv {
                    if let Some(old) = self.beacon.take() {
                        old.stop();
                    }
                    self.beacon = Some(beacon::Announcer::start(&code, &name, &srv));
                    self.beacon_srv = srv;
                }
            }
            None => self.end_beacon(),
        }
    }

    fn end_beacon(&mut self) {
        if let Some(b) = self.beacon.take() {
            b.stop();
        }
        self.beacon_srv.clear();
    }

    /// `L`: leave the room (hosts end the session for everyone).
    fn leave_room(&mut self) {
        if let Some(r) = self.room.take() {
            r.leave();
        }
        self.end_beacon();
        if let Some(reach) = self.room_punch.take() {
            thread::spawn(move || punch::forget(&reach));
        }
        self.room_key.clear();
        self.room_timer = 0.0;
        if self.mode == Mode::Room {
            self.mode = Mode::Normal;
        }
        self.status = "left the listening room".into();
    }

    fn join_room(&mut self, server: &str, code: &str) -> Result<(), String> {
        if self.room.is_some() {
            return Err("you are already in a room. press L to leave first".into());
        }
        let me = self.display_name();
        let (etx, erx) = channel::<Event>();
        let bridge = self.tx.clone();
        thread::spawn(move || {
            while let Ok(e) = erx.recv() {
                if bridge.send(Msg::Together(e)).is_err() {
                    break;
                }
            }
        });
        match together::join(server, &me, code, etx) {
            Ok(h) => {
                self.room = Some(h);
                self.room_key.clear();
                self.room_timer = 0.0;
                self.status = format!("joining {code} on {server} ...");
                Ok(())
            }
            Err(e) => Err(e),
        }
    }

    /// Applies a host's state. Tracks come from the listener's own library
    /// (matched by tags, then file name) or stream from their original url.
    fn room_follow(&mut self, st: &RoomState) {
        let resolved: Vec<Track> = st.queue.iter().map(|s| share::resolve_track(s, &self.local)).collect();
        if resolved.is_empty() {
            self.pl.clear();
            self.pl_idx = 0;
            if let Some(r) = self.room.as_mut() {
                r.state = st.clone();
            }
            return;
        }
        let idx = self.pl_idx.min(resolved.len() - 1);
        let want = resolved[idx].clone();
        let missing = want.path.is_none() && want.url.is_none();
        let same = self.now.as_ref().map(|n| same_track(n, &want)).unwrap_or(false);
        self.pl = resolved;
        self.pl_idx = idx;
        if let Some(r) = self.room.as_mut() {
            r.state = st.clone();
        }

        if same && missing {
            return;
        }
        if !same {
            self.drop_prefetch();
            if missing {
                self.drop_stream();
                self.now = Some(want.clone());
                self.playing = false;
                if let Some(e) = self.engine.as_mut() {
                    e.pause();
                }
                self.status = format!("not in your library: {}", want.title);
                return;
            }
            self.play_any(want);
        }
        if st.playing != self.playing {
            self.set_playing(st.playing);
        }
        if st.playing && self.playing && (self.pos - st.pos).abs() > 1.5 {
            self.do_seek(st.pos);
        }
    }

    fn on_room_event(&mut self, e: Event) {
        match e {
            Event::Created { server, code, host, members, state } => {
                if let Some(r) = self.room.as_mut() {
                    r.code = code.clone();
                    // keep a multi-address list the router work already filled in
                    if self.room_punch.is_none() {
                        r.server = server;
                    }
                    r.host_name = host;
                    r.members = members;
                    r.state = state;
                    r.pending = false;
                    r.connected = true;
                }
                self.room_key.clear();
                self.room_timer = 0.0;
                self.status = format!("room {code} is live - press H for the invite, L to leave");
                self.refresh_beacon();
            }
            Event::Joined { server, code, host, members, state } => {
                if let Some(r) = self.room.as_mut() {
                    r.code = code.clone();
                    r.server = server;
                    r.host_name = host.clone();
                    r.members = members;
                    r.state = state.clone();
                    r.pending = false;
                    r.connected = true;
                }
                if self.following() {
                    self.room_follow(&state);
                    self.status =
                        format!("joined room {code} - following {host}. audio comes from your own files");
                } else {
                    self.status = format!("room {code} is live - press H for the invite, L to leave");
                }
            }
            Event::Sync { host, members, state } => {
                if let Some(r) = self.room.as_mut() {
                    r.host_name = host;
                    r.members = members;
                    r.state = state.clone();
                }
                if self.following() {
                    self.room_follow(&state);
                }
            }
            Event::Members { host, members } => {
                if let Some(r) = self.room.as_mut() {
                    r.host_name = host;
                    r.members = members;
                }
            }
            Event::Closed(m) => {
                self.room = None;
                self.end_beacon();
                if let Some(reach) = self.room_punch.take() {
                    thread::spawn(move || punch::forget(&reach));
                }
                self.room_key.clear();
                self.room_timer = 0.0;
                if self.mode == Mode::Room {
                    self.mode = Mode::Normal;
                }
                // arriving after an explicit leave just cleans up quietly
                if self.status.contains("left the listening room") {
                    return;
                }
                self.status = m;
            }
            Event::Error(m) => {
                let dropping = self
                    .room
                    .as_ref()
                    .map(|r| r.pending || m.contains("host ended"))
                    .unwrap_or(false);
                if dropping {
                    if let Some(r) = self.room.take() {
                        r.leave();
                    }
                    self.end_beacon();
                    if let Some(reach) = self.room_punch.take() {
                        thread::spawn(move || punch::forget(&reach));
                    }
                    self.room_key.clear();
                    self.room_timer = 0.0;
                    if self.mode == Mode::Room {
                        self.mode = Mode::Normal;
                    }
                }
                self.status = m;
            }
        }
    }

    /// Hosts push their state (play/pause, track, position) to the room.
    /// Guests just send a heartbeat so a dead connection is noticed.
    fn room_tick(&mut self, dt: f64) {
        let Some(r) = self.room.as_ref() else { return };
        if r.host {
            if r.pending {
                return;
            }
            let st = self.room_state_now();
            let key = format!(
                "{}|{}|{}|{}",
                st.index,
                st.playing,
                st.queue.len(),
                self.pos as i64
            );
            if key != self.room_key || self.room_timer >= 1.0 {
                self.room_key = key;
                self.room_timer = 0.0;
                if let Some(r) = self.room.as_ref() {
                    r.send_state(st);
                }
            } else {
                self.room_timer += dt;
            }
        } else {
            self.room_timer += dt;
            if self.room_timer >= 20.0 {
                self.room_timer = 0.0;
                if let Some(r) = self.room.as_ref() {
                    r.ping();
                }
            }
        }
    }

    // ---------- share playlist ----------

    /// Share code for the open playlist (or the selected one in the overview).
    /// Local file paths are not included: only title, artist, duration and
    /// online urls, so no private paths leave the machine.
    fn share_current(&mut self) {
        if self.room.is_some() {
            self.status = "leave the room first, then share (press L)".into();
            return;
        }
        let which = match self.open_list {
            Some(i) => Some(i),
            None if self.source == PLAYLISTS && self.sel < self.playlists.len() => Some(self.sel),
            None => None,
        };
        let Some(i) = which else {
            self.status = "open a playlist (source 5) and press S to share it".into();
            return;
        };
        let (name, code) = {
            let Some(pl) = self.playlists.get(i) else {
                self.status = "no playlist selected".into();
                return;
            };
            match share::encode_playlist(pl) {
                Ok(code) => (pl.name.clone(), code),
                Err(e) => {
                    self.status = format!("cannot share this playlist: {e}");
                    return;
                }
            }
        };
        self.share_title = name.clone();
        self.share_path = dirs::config_dir()
            .map(|d| d.join("enigmatic-player").join("shares"))
            .and_then(|d| share::save_code(&d, &name, &code));
        self.share_code = code;
        self.share_copied = false;
        self.mode = Mode::Share;
        self.status = String::new();
    }

    fn key_share(&mut self, k: KeyEvent) {
        match k.code {
            KeyCode::Esc => self.mode = Mode::Normal,
            KeyCode::Enter => {
                if share::copy_to_terminal(&self.share_code) {
                    self.share_copied = true;
                    self.status = "code sent to the terminal clipboard (OSC52)".into();
                } else if let Some(p) = &self.share_path {
                    self.status = format!(
                        "terminal ignored the clipboard copy - use {}",
                        p.to_string_lossy()
                    );
                } else {
                    self.status = "could not copy; paste the code yourself".into();
                }
            }
            _ => {}
        }
    }

    fn key_join(&mut self, k: KeyEvent) {
        match k.code {
            KeyCode::Esc => {
                self.mode = Mode::Normal;
                self.join_input.clear();
                self.join_err = None;
            }
            KeyCode::Up | KeyCode::Char('k') => {
                self.refresh_ads();
                if !self.room_ads.is_empty() {
                    self.room_pick = (self.room_pick + self.room_ads.len() - 1) % self.room_ads.len();
                }
            }
            KeyCode::Down | KeyCode::Char('j') => {
                self.refresh_ads();
                if !self.room_ads.is_empty() {
                    self.room_pick = (self.room_pick + 1) % self.room_ads.len();
                }
            }
            KeyCode::Enter => {
                if self.join_input.trim().is_empty() {
                    self.join_picked();
                } else {
                    self.submit_join();
                }
            }
            _ => {
                if edit_text(&mut self.join_input, &k) {
                    self.join_err = None;
                    while self.join_input.len() > 200_000 {
                        self.join_input.pop();
                    }
                }
            }
        }
    }

    /// Refresh the discovered LAN rooms and clamp the selection onto it.
    fn refresh_ads(&mut self) {
        self.room_ads = beacon::rooms();
        if self.room_pick >= self.room_ads.len() {
            self.room_pick = 0;
        }
    }

    /// Enter on an empty input: join the room selected in the list.
    fn join_picked(&mut self) {
        self.refresh_ads();
        let Some(ad) = self.room_ads.get(self.room_pick).cloned() else {
            self.join_err =
                Some("nothing heard on this network yet - paste the invite your friend sent (it starts with EROOM1:)".into());
            return;
        };
        let err = match self.join_room(&ad.server, &ad.code) {
            Ok(()) => {
                self.mode = Mode::Normal;
                self.join_input.clear();
                self.join_err = None;
                None
            }
            Err(e) => Some(e),
        };
        if let Some(e) = err {
            self.join_err = Some(format!("cannot join {}: {e}", ad.code));
            self.status = e;
        }
    }

    fn key_room(&mut self, k: KeyEvent) {
        match k.code {
            KeyCode::Esc => self.mode = Mode::Normal,
            KeyCode::Enter | KeyCode::Char('c') => {
                let invite = self.room.as_ref().map(|r| r.invite()).unwrap_or_default();
                if invite.is_empty() {
                    self.status = "still connecting - try again in a second".into();
                } else if share::copy_to_terminal(&invite) {
                    self.status = "invite sent to the terminal clipboard (OSC52)".into();
                } else {
                    self.status = "terminal ignored the clipboard copy - read it from the popup".into();
                }
            }
            KeyCode::Char('l') | KeyCode::Char('L') => self.leave_room(),
            _ => {}
        }
    }

    /// `U` submit: a playlist share code, a room invite, a bare room code,
    /// or the path to a `.elist.txt` file saved here earlier.
    fn submit_join(&mut self) {
        let text = self.join_input.trim().to_string();
        if text.is_empty() {
            self.join_err = Some("paste a playlist share code or a room invite first.".into());
            return;
        }
        let payload = if !text.starts_with("ELIST1:") && !text.starts_with("EROOM1:") && Path::new(&text).is_file() {
            match std::fs::read_to_string(&text) {
                Ok(s) => s.trim().to_string(),
                Err(e) => {
                    self.join_err = Some(format!("cannot read that file: {e}"));
                    return;
                }
            }
        } else {
            text
        };
        if let Ok(list) = share::decode_playlist(&payload) {
            self.import_playlist(list);
            self.mode = Mode::Normal;
            return;
        }
        if together::looks_like_invite(&payload) {
            match together::decode_invite(&payload) {
                Ok((server, code)) => {
                    self.join_room(&server, &code).unwrap_or_else(|e| {
                        self.join_err = Some(e.clone());
                        self.status = e;
                    });
                }
                Err(e) => {
                    self.join_err = Some(e.clone());
                    self.status = e;
                }
            }
            return;
        }
        let code = payload.to_uppercase();
        let ok = code.len() >= 4 && code.len() <= 8 && code.chars().all(|c| c.is_ascii_alphanumeric());
        if !ok {
            self.join_err = Some("that does not look like a playlist code or a room invite".into());
            return;
        }
        // A bare code needs an address: check the rooms heard on this network,
        // then any manually configured relay, and own up if neither exists.
        let fresh = beacon::rooms();
        if let Some(ad) = self
            .room_ads
            .iter()
            .find(|a| a.code == code)
            .or_else(|| fresh.iter().find(|a| a.code == code))
        {
            let ad = ad.clone();
            if let Err(e) = self.join_room(&ad.server, &ad.code) {
                self.join_err = Some(e.clone());
                self.status = e;
                return;
            }
            self.mode = Mode::Normal;
            return;
        }
        match self.relay_override() {
            Some(s) => {
                if let Err(e) = self.join_room(&s, &code) {
                    self.join_err = Some(e.clone());
                    self.status = e;
                    return;
                }
                self.mode = Mode::Normal;
            }
            None => {
                self.join_err = Some(
                    "a room code alone has no address - paste the full invite (it starts with EROOM1:), or wait until the room shows up in this list".into(),
                );
            }
        }
    }

    /// `import`: resolve the shared list against the local library and add it.
    fn import_playlist(&mut self, shared: share::SharedList) {
        let resolved = shared.resolve(&self.local);
        if resolved.is_empty() {
            self.join_err = Some("that shared playlist is empty.".into());
            return;
        }
        let missing = resolved.iter().filter(|t| t.path.is_none() && t.url.is_none()).count();
        let base = {
            let b: String = share::sanitize(&shared.name).trim().chars().take(40).collect();
            if b.is_empty() {
                "shared playlist".into()
            } else {
                b
            }
        };
        let mut name = base.clone();
        let mut k = 1;
        while self.playlists.iter().any(|p| p.name.to_lowercase() == name.to_lowercase()) {
            k += 1;
            name = format!("{base} {k}");
        }
        self.join_input.clear();
        self.join_err = None;
        self.playlists.push(Playlist { name: name.clone(), tracks: resolved });
        self.save_lists();
        self.source = PLAYLISTS;
        self.open_list = Some(self.playlists.len() - 1);
        self.sel = 0;
        self.scroll = 0;
        self.query.clear();
        self.status = if missing > 0 {
            format!(
                "imported \"{name}\" - {missing} track(s) are not in your local files"
            )
        } else {
            format!("imported \"{name}\"")
        };
    }

    pub fn open_settings(&mut self) {
        self.mode = Mode::Settings;
        self.set_tab = 0;
        self.theme_state.select(Some(self.theme));
        self.dir_input = self.cfg.dir.clone();
        self.dir_msg = None;
    }

    fn close_settings(&mut self) {
        self.mode = Mode::Normal;
        self.save();
    }

    fn key_settings(&mut self, k: KeyEvent) {
        match k.code {
            KeyCode::Esc => self.close_settings(),
            KeyCode::Tab => {
                self.set_tab = (self.set_tab + 1) % 4;
                self.on_tab();
            }
            KeyCode::BackTab => {
                self.set_tab = (self.set_tab + 3) % 4;
                self.on_tab();
            }
            _ => match self.set_tab {
                0 => self.key_themes(k),
                1 => self.key_dir(k),
                2 => self.key_devices(k),
                _ => {}
            },
        }
    }

    fn on_tab(&mut self) {
        if self.set_tab == 2 {
            self.load_devices();
        }
    }

    /// Reads the device list. The sound system may print warnings to the
    /// terminal while probing, so the screen is cleared afterwards.
    fn load_devices(&mut self) {
        let mut v = vec!["system default".to_string()];
        v.extend(audio::list_output_devices());
        let cur = if self.cfg.device.is_empty() {
            0
        } else {
            v.iter().position(|d| *d == self.cfg.device).unwrap_or(0)
        };
        self.devices = v;
        self.dev_state.select(Some(cur));
        self.needs_clear = true;
    }

    fn key_devices(&mut self, k: KeyEvent) {
        let n = self.devices.len();
        if n == 0 {
            return;
        }
        let cur = self.dev_state.selected().unwrap_or(0);
        match k.code {
            KeyCode::Up | KeyCode::Char('k') => self.dev_state.select(Some((cur + n - 1) % n)),
            KeyCode::Down | KeyCode::Char('j') => self.dev_state.select(Some((cur + 1) % n)),
            KeyCode::Enter => {
                let name = if cur == 0 { String::new() } else { self.devices[cur].clone() };
                self.set_device(name);
            }
            _ => {}
        }
    }

    /// Switch the sound output. The current track continues on the new device.
    fn set_device(&mut self, name: String) {
        let was = self.now.clone();
        let pos = self.pos;
        let was_playing = self.playing;
        self.drop_prefetch();
        self.drop_stream();
        self.engine = None; // release the old device first

        match Engine::new(&name) {
            Ok(e) => {
                self.engine = Some(e);
                self.cfg.device = name.clone();
                self.apply_audio();
                self.save();
                let label = if name.is_empty() { "system default" } else { name.as_str() };
                self.status = format!("output: {label}");
                if let Some(t) = was {
                    self.play_any(t);
                    if pos > 1.0 {
                        self.seek_to(pos);
                    }
                    if !was_playing {
                        self.toggle_play();
                    }
                    self.status = format!("output: {label}");
                }
            }
            Err(e) => {
                self.status = format!("cannot open that output: {e}");
                let old = self.cfg.device.clone();
                if let Ok(e2) = Engine::new(&old) {
                    self.engine = Some(e2);
                    self.apply_audio();
                }
            }
        }
        self.needs_clear = true;
    }

    fn key_themes(&mut self, k: KeyEvent) {
        let n = THEMES.len();
        match k.code {
            KeyCode::Up | KeyCode::Char('k') => self.theme = (self.theme + n - 1) % n,
            KeyCode::Down | KeyCode::Char('j') => self.theme = (self.theme + 1) % n,
            KeyCode::Enter => {
                self.close_settings();
                return;
            }
            _ => return,
        }
        self.theme_state.select(Some(self.theme));
    }

    fn key_dir(&mut self, k: KeyEvent) {
        match k.code {
            KeyCode::Down => {
                self.pick_i = (self.pick_i + 1) % PICKS.len();
                self.dir_input = PICKS[self.pick_i].to_string();
                self.dir_msg = None;
            }
            KeyCode::Up => {
                self.pick_i = (self.pick_i + PICKS.len() - 1) % PICKS.len();
                self.dir_input = PICKS[self.pick_i].to_string();
                self.dir_msg = None;
            }
            KeyCode::Enter => self.save_dir(),
            _ => {
                if edit_text(&mut self.dir_input, &k) {
                    self.dir_msg = None;
                }
            }
        }
    }

    fn save_dir(&mut self) {
        let v = self.dir_input.trim().to_string();
        let b = v.as_bytes();
        if v.is_empty() {
            self.dir_msg = Some(("Enter a folder path first.".into(), false));
            return;
        }
        let ok = v.starts_with('~')
            || v.starts_with('/')
            || (b.len() >= 3 && b[1] == b':' && (b[2] == b'\\' || b[2] == b'/'));
        if !ok {
            self.dir_msg = Some((
                "Start the path with ~, /, or a drive letter like C:\\.".into(),
                false,
            ));
            return;
        }
        let _ = std::fs::create_dir_all(expand(&v));
        self.cfg.dir = v;
        self.save();
        self.dir_msg = Some(("Saved. New downloads go to this folder.".into(), true));
        self.rescan();
    }

    // ---------- mouse ----------

    pub fn on_mouse(&mut self, m: MouseEvent) {
        if self.mode != Mode::Normal && self.mode != Mode::Search {
            return;
        }
        let (x, y) = (m.column, m.row);
        match m.kind {
            MouseEventKind::ScrollUp => self.move_sel(-1),
            MouseEventKind::ScrollDown => self.move_sel(1),
            MouseEventKind::Down(MouseButton::Left) => self.click(x, y, true),
            MouseEventKind::Drag(MouseButton::Left) => self.click(x, y, false),
            _ => {}
        }
    }

    fn click(&mut self, x: u16, y: u16, down: bool) {
        if inside(self.hit.progress, x, y) {
            let f = frac(self.hit.progress, x) as f64;
            if self.dur > 0.0 {
                self.seek_to(f * self.dur);
            }
            return;
        }
        if inside(self.hit.vol, x, y) {
            let f = frac(self.hit.vol, x);
            self.set_vol((f * NB_VOL as f32).round() / NB_VOL as f32);
            return;
        }
        if inside(self.hit.rv_bar, x, y) {
            let f = frac(self.hit.rv_bar, x);
            self.set_wet((f * 100.0).round() as u32);
            return;
        }
        if !down {
            return;
        }
        if inside(self.hit.gear, x, y) {
            self.open_settings();
            return;
        }
        if inside(self.hit.mute, x, y) {
            self.toggle_mute();
            return;
        }
        if inside(self.hit.lib_header, x, y) {
            self.lib_open = !self.lib_open;
            return;
        }
        let btn = self.hit.btns.iter().find(|(r, _)| inside(*r, x, y)).map(|(_, b)| *b);
        if let Some(b) = btn {
            match b {
                Btn::Prev => self.prev(),
                Btn::Back => self.seek_by(-10.0),
                Btn::Play => self.toggle_play(),
                Btn::Fwd => self.seek_by(10.0),
                Btn::Next => self.next(1),
            }
            return;
        }
        let sp = self.hit.speeds.iter().find(|(r, _)| inside(*r, x, y)).map(|(_, s)| *s);
        if let Some(s) = sp {
            self.set_speed(s);
            return;
        }
        let rv = self.hit.reverbs.iter().find(|(r, _)| inside(*r, x, y)).map(|(_, i)| *i);
        if let Some(i) = rv {
            self.set_reverb_preset(i);
            return;
        }
        let src = self.hit.sources.iter().find(|(r, _)| inside(*r, x, y)).map(|(_, i)| *i);
        if let Some(i) = src {
            self.set_source(i);
            return;
        }
        if inside(self.hit.tracks, x, y) {
            let idx = self.hit.tracks_offset + (y - self.hit.tracks.y) as usize;
            if idx < self.rows() {
                if idx == self.sel {
                    self.activate(true);
                } else {
                    self.sel = idx;
                }
            }
        }
    }
}

/// The address this app dials to reach its own relay. The relay binds the
/// wildcard address so guests can reach it, but the wildcard itself is not a
/// valid destination: Windows refuses to connect to 0.0.0.0.
fn self_addr(port: u16) -> String {
    format!("127.0.0.1:{port}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn own_relay_is_dialled_on_loopback() {
        // Windows rejects a connection to the unspecified address, so the
        // hosting path must hand the client a real address.
        assert_eq!(self_addr(18199), "127.0.0.1:18199");
        assert!(!self_addr(18199).starts_with("0.0.0.0"));
    }

    /// The real hosting path: the relay listens on the wildcard address and the
    /// host client dials loopback. This failed with "cannot reach sync server
    /// 0.0.0.0:port" before the address was fixed.
    #[test]
    fn embedded_relay_is_reachable_on_loopback() {
        let listener = std::net::TcpListener::bind("0.0.0.0:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            let _ = crate::server::serve(listener);
        });
        let (etx, erx) = std::sync::mpsc::channel();
        let _host = crate::together::create(
            &self_addr(port),
            "Host",
            crate::together::RoomState::default(),
            etx,
        )
        .unwrap();
        match erx.recv_timeout(std::time::Duration::from_secs(5)) {
            Ok(crate::together::Event::Created { code, .. }) => {
                assert_eq!(code.len(), 8);
                assert!(code.chars().all(|c| c.is_ascii_digit()));
            }
            other => panic!("hosting did not create a room: {other:?}"),
        }
    }
}