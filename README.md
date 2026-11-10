# Enigmatic Player

Enigmatic Player is a pixel-terminal music player written in Rust for the desktop.

## Download

Windows (x86_64) builds are attached to the [latest release](https://github.com/lmdelm-dev/Enigmatic-Player/releases/latest): download `enigmatic-player-v1.8.0-windows-x64.exe` and run it in Windows Terminal, or the `.zip` if you also want the README and license. No installer and no runtime needed. On other platforms, build from source with `cargo build --release`.

## Rust desktop edition

The player lives in `enigmatic-player-rust/`. It uses native audio devices, terminal mouse regions, filesystem scanning, and an ffmpeg-backed stream pipeline.

### Run

```bash
cd enigmatic-player-rust
cargo run
```

See the source and `Cargo.toml` for native requirements and build instructions.

## Share a playlist

On the **playlists** source (press `5`), select a playlist and press `S`:

- A `ELIST1:` share code is copied/saved; the player also writes `~/.config/enigmatic-player/shares/<name>.elist.txt` (Windows: `%APPDATA%\enigmatic-player\shares\`).
- Another person presses `U`, pastes the code (or the path to that file), and gets a playlist import.
- The code contains **metadata and online URLs only** — no file paths leave your machine. For local tracks the importer's own library is matched by tags, then file name; import reports how many tracks were not found.

## Listen together (zero setup)

A room keeps everyone on the same track, position and play/pause state in real time. Audio is **not** streamed between people: every listener plays their own copy (local file, matched by tags/filename, or the same online URL).

Since v1.8.0 there is no server to run and nothing to configure:

1. The host presses `H` (or, on source 5, selects a playlist and presses `H`). The app **becomes the room itself**: it opens a room relay inside the process.
2. Automatically, in the background, the app tries to make the room reachable — first **NAT-PMP**, then **UPnP** — asking your router to open a mapping for the session only. When the router agrees (most consumer routers do, via UPnP), the invite also works **on the internet**.
3. **LAN discovery**: while a room is open the app announces it on the local network (UDP broadcast, nothing identifying beyond host name, room code and room addresses). Anyone else running the player on the same Wi-Fi/segment presses `U` and sees the room in a list — pick it, press enter, you are in. No typing at all.
4. Friends outside your network: they paste the invite `EROOM1:...` (copyable in the room popup). The invite carries **several addresses** (LAN, then the router-mapped public one); the joiner tries each in order, so the same single invite works for both friends at home and friends far away.
5. Press `L` to leave (hosts end the session for everyone and the router mapping is removed again).

While in a room, only the host changes tracks, seeks or pauses; guests see that in the room popup and the title bar (`live: CODE guest`).

Manual fallbacks, only if you want them:

- `--serve` still exists: `cargo run --release -- --serve 0.0.0.0:8080` runs a standalone relay; set `ENIGMATIC_SERVER=host:port` (env var) to point rooms at it instead of embedded hosting. A single invite may also carry several relay addresses separated by `|`.
- `ENIGMATIC_NO_PUNCH=1` disables the router port-opening entirely if you prefer your network untouched.

Important limits, stated honestly:

- Room traffic is plain TCP + JSON lines over the sync link: **no TLS, no accounts**. The 8-digit room code is the only access control. The relay holds room state only (track titles, artists, durations, online URLs) — never your audio files, and no local file paths ever leave the machine.
- When the router refuses both NAT-PMP and UPnP (some ISPs use CGNAT; some routers ship UPnP off), the room stays LAN-only and the app says so plainly instead of pretending. In that case the `--serve` fallback above is the way out for internet guests.
- **First run on Windows**: opening the port and receiving broadcasts can trigger a Windows Defender Firewall prompt — allow it for private networks, or discovery/port-opening will silently fail on that machine.
- Some Wi-Fi access points filter UDP broadcasts; then LAN discovery shows nothing, but pasting the invite always still works.
- Rooms are in memory only; leaving, quitting or a router reboot ends them.
- No server is operated by or bundled with this project at any public address; your friend's laptop is the server.

## Web edition

The browser edition (`index.html`, `app.js`, `styles.css`) recreates the compact terminal layout and the core controls: the same 33 themes, local files and folders, play/pause, seek, volume, shuffle and repeat, playback speed, reverb through Web Audio, a live spectrum visualizer, search, playlists and keyboard shortcuts.

Serve the repository root with any static web server, or publish it with GitHub Pages:

```bash
python -m http.server 8080
```

Local playback needs no backend; browsers cannot scan the disk silently, so music is granted through **+ files** or **+ folder**. A copy of the browser player also lives in `enigmatic-player-rust/web/`.

### Online sources backend

`backend/` is a small FastAPI + yt-dlp service that resolves YouTube and SoundCloud search results (Bandcamp works by pasting a track or album URL). Set its URL in the web player's **Settings**; the health endpoint is `/health`. Deploy with `backend/Dockerfile` or `render.yaml`, and set `ALLOWED_ORIGINS` to the published origin instead of the local-testing default `*`.

Browser limits, stated honestly: provider URL expiry, CORS, codec support and rate limits all affect playback, so third-party streaming is never permanent. The desktop edition uses native audio devices, terminal mouse regions, filesystem scanning and an ffmpeg-backed stream pipeline — none of which a static page can reproduce.

## License

See the repository license file.
