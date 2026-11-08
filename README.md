# Enigmatic Player (Rust)

A pixel-style terminal music player. Real audio, a real spectrum visualizer, seekable progress bar, block volume, speed, live reverb, search, 33 themes, and a settings window. Made by Jamal Tiliouan. Open source.

> Status: v1.5. The streaming and performance changes were written without a compiler. Run `cargo check` first. If it reports errors, they should be small. See `AGENTS.md` for a prompt that fixes them.

## Requirements

- Rust (stable). Install from https://rustup.rs
- A terminal with true color and at least 80x28 characters
- Linux only: ALSA headers for audio output

```
# Debian / Ubuntu
sudo apt install libasound2-dev pkg-config
# Fedora
sudo dnf install alsa-lib-devel
```

Optional, for YouTube, SoundCloud and Bandcamp:

- `yt-dlp` (search, streaming and saving)
- `ffmpeg` (decodes streams, converts saved tracks to mp3)

Keep yt-dlp up to date (`yt-dlp -U` or `pip install -U yt-dlp`). Most "service busy" and failed-search problems come from an old yt-dlp.

## Run

```
cargo run --release
```

On first start it scans `~/Music/enigmatic-player`. Change the folder in settings (`s`). Put music files there (mp3, flac, ogg, wav, m4a, aac).

## Keys

| Key | Action |
| --- | --- |
| `space` | Play or pause |
| `left` / `right` (or `h` / `l`) | Seek 5 seconds |
| `up` / `down` (or `k` / `j`) | Move in the list |
| `enter` | Play. Online tracks stream, no download needed first |
| `d` | Save an online track into your music folder as high-quality mp3 |
| `/` | Search. Local search filters live; online sources search on `enter` |
| `tab` | Hide or show the library |
| `1` to `4` | Source: local, YouTube, SoundCloud, Bandcamp |
| `-` / `=` | Volume |
| `m` | Mute |
| `[` / `]` | Speed down and up by 0.1 (0.5x to 2.0x, also changes pitch like tape) |
| `r` | Cycle reverb preset |
| `<` / `>` | Reverb amount |
| `n` / `p` | Next and previous |
| `s` | Settings |
| `q` | Quit |

Mouse: click or drag the progress bar, volume bar and reverb bar. Click the transport buttons, speed chips, reverb chips, sources, tracks (click a selected track to play), the library header, and the settings label.

## Settings

- **themes**: 33 themes, previewed live as you move through the list
- **local music**: the folder for your library and downloads
- **about**: version, author, shortcuts

Settings are saved to `config.json` in your config folder (`~/.config/enigmatic-player/` on Linux).

## Online sources

- YouTube and SoundCloud: choose the source, press `/`, type, press `enter`.
- Bandcamp has no search in yt-dlp. Paste a track or album URL after `/`.
- `enter` on a result **streams** it: yt-dlp fetches the best audio, ffmpeg decodes it to the sound card's sample rate, and playback starts after about a second of audio is ready. You can seek inside the part that has already arrived. `next` and `previous` keep streaming through the results.
- `d` saves the track to your music folder as mp3 (best quality), in the background. You can save several at once while music plays.
- If a service answers "busy", "429" or times out, the player retries up to 3 times and shows "service busy, retrying (2/3)".
- Search results are cached for the session, so repeating a search is instant, and a new search replaces any search still running.
- Streams use temporary files in your cache folder. They are deleted when you change track and when you quit.

Respect each service's terms of use.

## Performance

- Redraws only when something changed, and sleeps longer when paused, so it idles near zero CPU.
- Mouse movement is ignored; only clicks, drags and scrolling cause redraws.
- The library scan is cached (`library.json` in your cache folder) and new files are read in parallel, so startup is fast even for big libraries.
- The track list only builds the rows you can see.
- Reverb costs nothing while it is off.
- Release builds use full link-time optimization and are stripped. Build with `cargo run --release`.

## Audio quality

- Streams are resampled by ffmpeg to your sound card's rate, which avoids a lower-quality second resample.
- Reverb uses damped comb filters plus allpass diffusers with a stereo spread, and a soft limiter instead of hard clipping.
- Saved mp3 files use the highest VBR quality.
- Local files are resampled by the audio library when their rate differs from the card.

## Project layout

| File | Purpose |
| --- | --- |
| `src/main.rs` | Terminal setup and main loop |
| `src/app.rs` | State, keyboard, mouse, background jobs |
| `src/ui.rs` | All drawing (ratatui) |
| `src/audio.rs` | Playback engine, live reverb, spectrum analyzer, stream reader |
| `src/stream.rs` | yt-dlp to ffmpeg streaming pipeline with retries |
| `src/library.rs` | Folder scan, tags, yt-dlp search and download |
| `src/config.rs` | Saved settings |
| `src/themes.rs` | The 33 themes |
