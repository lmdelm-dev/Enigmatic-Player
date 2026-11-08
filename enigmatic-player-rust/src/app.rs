use std::collections::HashMap;
use std::sync::atomic::Ordering;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::thread;

use ratatui::{
    crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind},
    layout::Rect,
    widgets::ListState,
};

use crate::audio::{self, Engine};
use crate::config::{expand, Config};
use crate::library::{self, Provider, Track};
use crate::stream::{self, Stream};
use crate::themes::{self, THEMES};

pub const REVERBS: [(&str, u32); 5] = [("off", 0), ("room", 25), ("hall", 45), ("cave", 65), ("space", 85)];
pub const SPEEDS: [f32; 16] = [
    0.5, 0.6, 0.7, 0.8, 0.9, 1.0, 1.1, 1.2, 1.3, 1.4, 1.5, 1.6, 1.7, 1.8, 1.9, 2.0,
];
pub const NB_VOL: usize = 16;
pub const NB_RV: usize = 8;
pub const BANDS: usize = 14;
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
    pub hit: Hit,
    pub quit: bool,
    search_gen: u64,
    cache: HashMap<SearchKey, Vec<Track>>,
    tx: Sender<Msg>,
    rx: Receiver<Msg>,
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
            hit: Hit::default(),
            quit: false,
            search_gen: 0,
            cache: HashMap::new(),
            tx,
            rx,
        };
        app.apply_audio();
        app.rescan();
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
        self.drop_stream();
        stream::clear_cache();
        self.save();
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
        } else {
            self.remote.iter().collect()
        }
    }

    fn fix_sel(&mut self) {
        let n = self.list().len();
        if n == 0 {
            self.sel = 0;
        } else if self.sel >= n {
            self.sel = n - 1;
        }
    }

    fn move_sel(&mut self, d: isize) {
        let n = self.list().len();
        if n == 0 {
            self.sel = 0;
            return;
        }
        self.sel = (self.sel as isize + d).clamp(0, n as isize - 1) as usize;
    }

    fn set_source(&mut self, i: usize) {
        if i >= Provider::ALL.len() {
            return;
        }
        self.source = i;
        self.sel = 0;
        self.scroll = 0;
        if i != 0 && !self.have_ytdlp {
            self.status = "yt-dlp not found. Install it to use online sources.".into();
        } else if Provider::ALL[i] == Provider::Bandcamp {
            self.status = "bandcamp: press / and paste a track or album url".into();
        } else if i != 0 {
            self.status = "press / to search, then enter".into();
        } else {
            self.status.clear();
        }
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
            return;
        }

        // a new search replaces any search still running
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
        let s = match Stream::start(&url, rate, t.secs) {
            Ok(s) => s,
            Err(e) => {
                self.status = e;
                return;
            }
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
        self.playing = !self.playing;
        let playing = self.playing;
        if let Some(e) = self.engine.as_mut() {
            if playing {
                e.resume();
            } else {
                e.pause();
            }
        }
    }

    fn next(&mut self, d: isize) {
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
        if self.pos > 3.0 {
            self.seek_to(0.0);
        } else {
            self.next(-1);
        }
    }

    pub fn seek_to(&mut self, t: f64) {
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
                self.next(1);
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
        }
    }

    fn key_normal(&mut self, k: KeyEvent) {
        match k.code {
            KeyCode::Char('q') => self.quit = true,
            KeyCode::Char('/') => self.mode = Mode::Search,
            KeyCode::Char('s') | KeyCode::Char(',') => self.open_settings(),
            KeyCode::Tab => self.lib_open = !self.lib_open,
            KeyCode::Char(' ') => self.toggle_play(),
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
            KeyCode::Char(c @ '1'..='4') => self.set_source(c as usize - '1' as usize),
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
                if self.source != 0 {
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
        if self.mode == Mode::Settings {
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
            if idx < self.list().len() {
                if idx == self.sel {
                    self.activate(true);
                } else {
                    self.sel = idx;
                }
            }
        }
    }
}
