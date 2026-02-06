use std::fs::File;
use std::io::{BufReader, Read, Seek, SeekFrom};
use std::path::Path;
use std::sync::atomic::{AtomicU32, AtomicU64, AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use rodio::cpal::traits::{DeviceTrait, HostTrait};
use rodio::source::SeekError;
use rodio::{Decoder, OutputStream, OutputStreamHandle, Sink, Source};

use crate::stream::Stream;

const COMB_MS: [f32; 4] = [29.7, 37.1, 41.1, 43.7];
const AP_MS: [f32; 2] = [12.6, 10.0];
const SPREAD: usize = 23; // extra delay (samples) per channel, widens the stereo image
// (delay scale, feedback, damping) for: off, room, hall, cave, space
const PRESETS: [(f32, f32, f32); 5] = [
    (1.0, 0.70, 0.25),
    (1.0, 0.70, 0.25),
    (1.6, 0.78, 0.35),
    (2.4, 0.84, 0.50),
    (3.2, 0.88, 0.40),
];
const TAP_KEEP: usize = 2048;
const FFT_N: usize = 1024;

/// Effect settings shared between the UI thread and the audio thread.
pub struct Fx {
    pub wet: AtomicU32,
    pub preset: AtomicU32,
}

/// Gentle limiter: transparent below 0.8, smooth curve above, never clips hard.
fn soft(v: f32) -> f32 {
    let a = v.abs();
    if a < 0.8 {
        v
    } else {
        v.signum() * (0.8 + 0.2 * ((a - 0.8) / 0.2).tanh())
    }
}

/// Wraps a source: applies a live-adjustable reverb (damped combs + allpass
/// diffusers, only when it is switched on) and copies samples into a shared
/// buffer so the UI can draw a real spectrum.
pub struct FxSource<S: Source<Item = f32>> {
    inner: S,
    fx: Arc<Fx>,
    tap: Arc<Mutex<Vec<f32>>>,
    channels: u16,
    rate: u32,
    len: usize,
    combs: Vec<Vec<f32>>,
    lp: Vec<f32>,
    cw: Vec<usize>,
    aps: Vec<Vec<f32>>,
    apw: Vec<usize>,
    apd: [usize; 2],
    dl: Vec<[usize; 4]>,
    preset: usize,
    active: bool,
    ch_idx: usize,
    chunk: Vec<f32>,
}

impl<S: Source<Item = f32>> FxSource<S> {
    pub fn new(inner: S, fx: Arc<Fx>, tap: Arc<Mutex<Vec<f32>>>) -> Self {
        let channels = inner.channels().max(1);
        let rate = inner.sample_rate().max(8000);
        let ch = channels as usize;
        let len = (rate as f32 * 0.17) as usize + SPREAD * ch + 2;
        let alen = (rate as f32 * 0.015) as usize + 2;
        let apd = [
            ((AP_MS[0] / 1000.0) * rate as f32) as usize,
            ((AP_MS[1] / 1000.0) * rate as f32) as usize,
        ];
        FxSource {
            inner,
            fx,
            tap,
            channels,
            rate,
            len,
            combs: vec![vec![0.0; len]; ch * 4],
            lp: vec![0.0; ch * 4],
            cw: vec![0; ch],
            aps: vec![vec![0.0; alen]; ch * 2],
            apw: vec![0; ch * 2],
            apd,
            dl: vec![[1; 4]; ch],
            preset: 99,
            active: false,
            ch_idx: 0,
            chunk: Vec::with_capacity(512),
        }
    }

    fn recompute(&mut self, p: usize) {
        self.preset = p;
        let scale = PRESETS[p].0;
        for c in 0..self.channels as usize {
            for k in 0..4 {
                let d = ((COMB_MS[k] * scale / 1000.0) * self.rate as f32) as usize + c * SPREAD;
                self.dl[c][k] = d.clamp(1, self.len - 1);
            }
        }
    }

    fn reset(&mut self) {
        for b in self.combs.iter_mut() {
            b.iter_mut().for_each(|v| *v = 0.0);
        }
        for b in self.aps.iter_mut() {
            b.iter_mut().for_each(|v| *v = 0.0);
        }
        self.lp.iter_mut().for_each(|v| *v = 0.0);
    }
}

impl<S: Source<Item = f32>> Iterator for FxSource<S> {
    type Item = f32;

    fn next(&mut self) -> Option<f32> {
        let x = self.inner.next()?;
        let c = self.ch_idx;
        let wet = self.fx.wet.load(Ordering::Relaxed) as f32 / 100.0;

        let out = if wet > 0.001 {
            if !self.active {
                self.reset();
                self.active = true;
            }
            let p = (self.fx.preset.load(Ordering::Relaxed) as usize).min(PRESETS.len() - 1);
            if p != self.preset {
                self.recompute(p);
            }
            let (_, fb, damp) = PRESETS[p];
            let len = self.len;
            let wp = self.cw[c];
            let input = x * (1.0 - fb) * 2.0;
            let mut acc = 0.0f32;

            for k in 0..4 {
                let delay = self.dl[c][k];
                let rp = if wp >= delay { wp - delay } else { wp + len - delay };
                let idx = c * 4 + k;
                let y = self.combs[idx][rp];
                let l = y * (1.0 - damp) + self.lp[idx] * damp;
                self.lp[idx] = l;
                self.combs[idx][wp] = input + l * fb;
                acc += y;
            }
            self.cw[c] = if wp + 1 >= len { 0 } else { wp + 1 };

            let mut w = acc * 0.25;
            for a in 0..2 {
                let idx = c * 2 + a;
                let alen = self.aps[idx].len();
                let ap = self.apw[idx];
                let d = self.apd[a].clamp(1, alen - 1);
                let rp = if ap >= d { ap - d } else { ap + alen - d };
                let bo = self.aps[idx][rp];
                self.aps[idx][ap] = w + bo * 0.5;
                w = -w + bo;
                self.apw[idx] = if ap + 1 >= alen { 0 } else { ap + 1 };
            }

            soft(x * (1.0 - 0.35 * wet) + w * wet * 1.6)
        } else {
            self.active = false;
            x
        };

        if c == 0 {
            self.chunk.push(out);
            if self.chunk.len() >= 512 {
                if let Ok(mut b) = self.tap.try_lock() {
                    b.extend_from_slice(&self.chunk);
                    let n = b.len();
                    if n > TAP_KEEP {
                        b.drain(0..n - TAP_KEEP);
                    }
                }
                self.chunk.clear();
            }
        }

        self.ch_idx = if c + 1 >= self.channels as usize { 0 } else { c + 1 };
        Some(out)
    }
}

impl<S: Source<Item = f32>> Source for FxSource<S> {
    fn current_frame_len(&self) -> Option<usize> {
        self.inner.current_frame_len()
    }
    fn channels(&self) -> u16 {
        self.channels
    }
    fn sample_rate(&self) -> u32 {
        self.rate
    }
    fn total_duration(&self) -> Option<Duration> {
        self.inner.total_duration()
    }
    fn try_seek(&mut self, pos: Duration) -> Result<(), SeekError> {
        self.inner.try_seek(pos)?;
        self.ch_idx = 0;
        self.chunk.clear();
        self.reset();
        Ok(())
    }
}

/// Reads the growing raw PCM file written by the stream pipeline.
/// Outputs silence while waiting for data, so a slow network only pauses the sound.
pub struct PcmSource {
    file: File,
    written: Arc<AtomicU64>,
    done: Arc<AtomicBool>,
    consumed: Arc<AtomicU64>,
    rate: u32,
    secs: f64,
    fpos: u64,
    buf: Vec<u8>,
    len: usize,
    idx: usize,
    local: u64,
}

impl PcmSource {
    pub fn new(s: &Stream) -> std::io::Result<PcmSource> {
        Ok(PcmSource {
            file: File::open(&s.path)?,
            written: s.written.clone(),
            done: s.done.clone(),
            consumed: s.consumed.clone(),
            rate: s.rate,
            secs: s.secs,
            fpos: 0,
            buf: vec![0u8; 32768],
            len: 0,
            idx: 0,
            local: 0,
        })
    }
}

impl Iterator for PcmSource {
    type Item = f32;

    fn next(&mut self) -> Option<f32> {
        if self.idx + 2 > self.len {
            let avail = self.written.load(Ordering::Acquire).saturating_sub(self.fpos);
            let n = ((avail.min(self.buf.len() as u64) / 4) * 4) as usize;
            if n == 0 {
                if self.done.load(Ordering::Acquire) {
                    let again = self.written.load(Ordering::Acquire).saturating_sub(self.fpos);
                    if again < 4 {
                        return None;
                    }
                }
                return Some(0.0);
            }
            if self.file.read_exact(&mut self.buf[..n]).is_err() {
                return Some(0.0);
            }
            self.fpos += n as u64;
            self.len = n;
            self.idx = 0;
        }
        let v = i16::from_le_bytes([self.buf[self.idx], self.buf[self.idx + 1]]);
        self.idx += 2;
        self.local += 1;
        if self.local & 1023 == 0 {
            self.consumed.store(self.local, Ordering::Relaxed);
        }
        Some(v as f32 / 32768.0)
    }
}

impl Source for PcmSource {
    fn current_frame_len(&self) -> Option<usize> {
        None
    }
    fn channels(&self) -> u16 {
        2
    }
    fn sample_rate(&self) -> u32 {
        self.rate
    }
    fn total_duration(&self) -> Option<Duration> {
        if self.secs > 0.0 {
            Some(Duration::from_secs_f64(self.secs))
        } else {
            None
        }
    }
    fn try_seek(&mut self, pos: Duration) -> Result<(), SeekError> {
        let frames = (pos.as_secs_f64() * self.rate as f64) as u64;
        let bytes = frames * 4;
        self.fpos = bytes;
        self.len = 0;
        self.idx = 0;
        self.local = frames * 2;
        self.consumed.store(self.local, Ordering::Relaxed);
        let _ = self.file.seek(SeekFrom::Start(bytes));
        Ok(())
    }
}

/// Names of all output devices the sound system reports.
pub fn list_output_devices() -> Vec<String> {
    rodio::cpal::default_host()
        .output_devices()
        .map(|it| it.filter_map(|d| d.name().ok()).collect())
        .unwrap_or_default()
}

fn pick_device(name: &str) -> Option<rodio::cpal::Device> {
    let host = rodio::cpal::default_host();
    if !name.is_empty() {
        if let Ok(mut it) = host.output_devices() {
            if let Some(d) = it.find(|d| d.name().map(|n| n == name).unwrap_or(false)) {
                return Some(d);
            }
        }
    }
    host.default_output_device()
}

pub struct Engine {
    _stream: OutputStream,
    handle: OutputStreamHandle,
    sink: Option<Sink>,
    fx: Arc<Fx>,
    tap: Arc<Mutex<Vec<f32>>>,
    rate: u32,
    out_rate: u32,
    volume: f32,
    speed: f32,
    paused: bool,
    stream: Option<Stream>,
    window: Vec<f32>,
}

impl Engine {
    /// `device` is an output device name, or empty for the system default.
    pub fn new(device: &str) -> Result<Engine, String> {
        let dev = pick_device(device).ok_or_else(|| "no audio output device".to_string())?;
        let out_rate = dev
            .default_output_config()
            .ok()
            .map(|c| c.sample_rate().0)
            .unwrap_or(48000);
        let (stream, handle) = OutputStream::try_from_device(&dev).map_err(|e| e.to_string())?;
        let window = (0..FFT_N)
            .map(|i| 0.5 - 0.5 * (2.0 * std::f32::consts::PI * i as f32 / (FFT_N as f32 - 1.0)).cos())
            .collect();
        Ok(Engine {
            _stream: stream,
            handle,
            sink: None,
            fx: Arc::new(Fx {
                wet: AtomicU32::new(0),
                preset: AtomicU32::new(0),
            }),
            tap: Arc::new(Mutex::new(Vec::new())),
            rate: 44100,
            out_rate,
            volume: 0.7,
            speed: 1.0,
            paused: false,
            stream: None,
            window,
        })
    }

    /// Sample rate of the sound card. Streams are resampled to this by ffmpeg.
    pub fn out_rate(&self) -> u32 {
        self.out_rate
    }

    fn start(&mut self, path: &Path, skip: Option<Duration>) -> Result<(), String> {
        let file = File::open(path).map_err(|e| e.to_string())?;
        let dec = Decoder::new(BufReader::new(file)).map_err(|e| e.to_string())?;
        self.rate = dec.sample_rate();
        let sink = Sink::try_new(&self.handle).map_err(|e| e.to_string())?;
        sink.set_volume(self.volume);
        sink.set_speed(self.speed);
        let base = dec.convert_samples::<f32>();
        match skip {
            Some(d) => {
                let s = base.skip_duration(d);
                sink.append(FxSource::new(s, self.fx.clone(), self.tap.clone()));
            }
            None => {
                sink.append(FxSource::new(base, self.fx.clone(), self.tap.clone()));
            }
        }
        if self.paused {
            sink.pause();
        }
        if let Some(old) = self.sink.replace(sink) {
            old.stop();
        }
        Ok(())
    }

    pub fn play(&mut self, path: &Path) -> Result<(), String> {
        self.paused = false;
        self.stream = None;
        let r = self.start(path, None);
        if let Ok(mut b) = self.tap.lock() {
            b.clear();
        }
        r
    }

    pub fn play_stream(&mut self, s: &Stream) -> Result<(), String> {
        let src = PcmSource::new(s).map_err(|e| e.to_string())?;
        let sink = Sink::try_new(&self.handle).map_err(|e| e.to_string())?;
        sink.set_volume(self.volume);
        sink.set_speed(self.speed);
        sink.append(FxSource::new(src, self.fx.clone(), self.tap.clone()));
        self.paused = false;
        self.rate = s.rate;
        self.stream = Some(s.clone());
        if let Some(old) = self.sink.replace(sink) {
            old.stop();
        }
        if let Ok(mut b) = self.tap.lock() {
            b.clear();
        }
        Ok(())
    }

    pub fn is_stream(&self) -> bool {
        self.stream.is_some()
    }

    /// Exact position of a streamed track, from the samples actually played.
    pub fn stream_pos(&self) -> Option<f64> {
        self.stream
            .as_ref()
            .map(|s| s.consumed.load(Ordering::Relaxed) as f64 / (2.0 * s.rate as f64))
    }

    pub fn seek_stream(&mut self, t: f64) -> bool {
        let d = Duration::from_secs_f64(t.max(0.0));
        match &self.sink {
            Some(s) => s.try_seek(d).is_ok(),
            None => false,
        }
    }

    /// Seek a file to `t` seconds. Tries a native seek first, then re-opens the file and skips ahead.
    pub fn seek(&mut self, path: &Path, t: f64) -> bool {
        let d = Duration::from_secs_f64(t.max(0.0));
        if let Some(s) = &self.sink {
            if s.try_seek(d).is_ok() {
                return true;
            }
        }
        self.start(path, Some(d)).is_ok()
    }

    pub fn pause(&mut self) {
        self.paused = true;
        if let Some(s) = &self.sink {
            s.pause();
        }
    }

    pub fn resume(&mut self) {
        self.paused = false;
        if let Some(s) = &self.sink {
            s.play();
        }
    }

    pub fn set_volume(&mut self, v: f32) {
        self.volume = v.clamp(0.0, 1.5);
        if let Some(s) = &self.sink {
            s.set_volume(self.volume);
        }
    }

    /// Playback speed. Like a tape, this also changes pitch.
    pub fn set_speed(&mut self, v: f32) {
        self.speed = v.clamp(0.25, 3.0);
        if let Some(s) = &self.sink {
            s.set_speed(self.speed);
        }
    }

    pub fn set_reverb(&mut self, preset: usize, wet: u32) {
        self.fx.preset.store(preset as u32, Ordering::Relaxed);
        self.fx.wet.store(wet.min(100), Ordering::Relaxed);
    }

    pub fn finished(&self) -> bool {
        self.sink.as_ref().map(|s| s.empty()).unwrap_or(false)
    }

    /// Real spectrum of the audio that is playing, `bands` values in 0..1.
    pub fn spectrum(&self, bands: usize) -> Vec<f32> {
        let n = self.window.len();
        let samples: Vec<f32> = match self.tap.lock() {
            Ok(b) if b.len() >= n => b[b.len() - n..].to_vec(),
            _ => return vec![0.0; bands],
        };
        let sr = self.rate.max(8000) as f32;
        (0..bands)
            .map(|k| {
                let f = 60.0 * (12000.0f32 / 60.0).powf(k as f32 / (bands as f32 - 1.0));
                let f = f.min(sr * 0.45);
                let w = 2.0 * std::f32::consts::PI * f / sr;
                let coeff = 2.0 * w.cos();
                let (mut s1, mut s2) = (0.0f32, 0.0f32);
                for (i, x) in samples.iter().enumerate() {
                    let s = x * self.window[i] + coeff * s1 - s2;
                    s2 = s1;
                    s1 = s;
                }
                let power = (s1 * s1 + s2 * s2 - coeff * s1 * s2).max(0.0);
                let mag = power.sqrt() / (n as f32 / 4.0);
                let db = 20.0 * (mag + 1e-6).log10();
                ((db + 60.0) / 50.0).clamp(0.0, 1.0)
            })
            .collect()
    }
}
