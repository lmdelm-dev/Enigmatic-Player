# AGENTS.md - guide for AI coding assistants

Project: Enigmatic Player, a pixel-style terminal music player in Rust.
Version 1.6. Stack: ratatui 0.28 (re-exports crossterm as `ratatui::crossterm`), rodio 0.19 with symphonia, lofty 0.21 for tags, walkdir, serde, dirs.

## First task: make it compile

This code was written without a compiler. Do this first:

1. Run `cargo check`.
2. Fix every error with the smallest possible change. Do not redesign anything.
3. Run `cargo clippy` and fix warnings only if trivial.
4. Run `cargo run --release` and test: play a file, seek, change volume, speed and reverb, open settings, change theme.

Most likely trouble spots, because crate APIs differ between versions:
- `lofty` imports in `src/library.rs` (`lofty::prelude::*`, `lofty::read_from_path`)
- `rodio` types in `src/audio.rs` (`SeekError` path, `Sink::try_seek`, `skip_duration`)
- ratatui calls in `src/ui.rs` (`f.area()`, `ListState::offset()`)

If a crate version is the problem, change the version in `Cargo.toml` instead of rewriting code.

## Architecture

- `main.rs`: raw mode, alternate screen, mouse capture, a 50 ms loop calling `ui::draw`, input handlers and `App::tick`.
- `app.rs`: `App` holds all state. Keyboard in `key_normal`, `key_search`, `key_settings`. Mouse in `click`, using the `Hit` rectangles that `ui.rs` refreshes each frame. Slow work (folder scan, yt-dlp search, download) runs in threads and reports through an mpsc channel (`Msg`), polled in `tick`.
- `ui.rs`: pure drawing. It also records clickable rectangles into `app.hit`. Colors come only from the active `Theme`.
- `stream.rs`: online playback. yt-dlp (stdout) -> ffmpeg -> raw s16le PCM appended to a cache file by a worker thread. Shared atomics: `written` (bytes), `consumed` (samples played), `done`, `kill`. Retries on transient errors (`library::is_transient`). `audio::PcmSource` reads the growing file, outputs silence while waiting, supports seeking by byte offset.
- `audio.rs`: `Engine` owns the rodio sink. `FxSource` wraps the decoded audio: a four-comb reverb whose amount and preset are shared atomics (live changes), plus a tap that feeds a spectrum analyzer (Goertzel over 1024 samples, 14 bands). Speed uses `Sink::set_speed` (changes pitch). Seeking tries `try_seek`, then falls back to re-opening the file and skipping.
- `library.rs`: scan, tags, yt-dlp search and download.
- `playlists.rs`: `Playlist { name, tracks }`, saved as JSON through a temp file. In `app.rs`, playlists are source index `PLAYLISTS` (4) in `Provider::ALL`. `open_list: Option<usize>` is the opened playlist (None = the list of playlists). `rows()` is the row count of the current view, `list()` the tracks. Modes `Pick` (choose where to add) and `Name` (new/rename) drive two popups drawn in `ui.rs`.
- `config.rs`: JSON config in the user config folder.
- `themes.rs`: 33 themes, each `[bg, panel, sel, line, text, dim, red, cyan, green, purple, yellow]`.

## Conventions

- No hardcoded colors in `ui.rs`. Use the theme fields.
- Keep the pixel look: block characters, thick borders, lowercase labels.
- Any change to the playlists view must keep `rows()`, `list()` and the drawing code in `draw_library` consistent.
- Keep keyboard shortcuts in sync in three places: `key_normal`, the About tab in `ui.rs`, and `README.md`.
- Never block the UI thread. Use a thread and a `Msg`.
- Redraw only when `App::tick` or an input event says something changed (`dirty` in `main.rs`). Do not add per-frame work that runs while idle.
- Do not build UI rows for the whole library; only the visible window.
- Minimum terminal size is 80x28. Keep layouts working at that size.

## Roadmap

1. Compile and test (above).
2. Tune the reverb levels by ear (input gain and mix in `FxSource::next`).
3. Playlist queue view and shuffle/repeat (the status bar already reserves space).
4. Album art from tags, shown with half-block pixels.
5. MPRIS / media keys on Linux.
6. Playlist extras: play shuffled, import/export m3u, per-playlist play counts.
7. Unit tests for `library::scan`, `config` and the reverb source.

## Prompts to paste

Compile:
> Read AGENTS.md. Run cargo check and fix all compile errors with minimal changes. Keep behavior and layout the same.

Shuffle and repeat:
> Add shuffle (key `x`) and repeat modes (key `y`: off, all, one). Show them in the status bar. Keep the existing pl and pl_idx logic working.

Streaming test:
> Test online playback end to end with yt-dlp and ffmpeg installed. Check start time, seeking inside the buffered part, next/previous, and cancelling by changing track. Fix any race in `stream.rs` and `PcmSource`.

## Do not

- Do not add async runtimes or a GUI toolkit.
- Do not change the theme data format.
- Do not remove the config file handling.
