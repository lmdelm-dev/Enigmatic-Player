# Enigmatic Player

Enigmatic Player is a pixel-terminal music player written in Rust for the desktop.

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

## Listen together by invite

A room keeps everyone on the same track, position and play/pause state in real time. Audio is **not** streamed between people: every listener plays their own copy (local file, matched by tags/filename, or the same online URL).

1. Someone runs a sync relay: `cargo run --release -- --serve 0.0.0.0:8080` (or `--serve` alone for `0.0.0.0:8080`).
2. The host presses `H` (or, on source 5, selects a playlist and presses `H`) to host on the server set under **Settings > online** (default `127.0.0.1:8080`, override with the `ENIGMATIC_SERVER` env var or by editing it there). An invite like `EROOM1:<hex-server>:<CODE>` is shown in the room popup (`H` anytime while live).
3. Guests press `U`, paste the invite (or just the room code plus the same server setting), and follow the host.

While in a room, only the host changes tracks, seeks or pauses; guests see that in the popup and in the title bar (`live: CODE guest`). Press `L` to leave (hosts end the session).

Important limits, stated honestly:

- The relay is plain TCP + JSON lines: **no TLS, no accounts**. The 6-character room code is the only access control. Run it on a trusted LAN, or behind a TLS reverse proxy — do not port-forward it to the public internet unprotected.
- Rooms are in memory only; leaving or restarting the server ends them.
- Key presses are not a substitute for a real chat; there is none.
- No server is operated by or bundled with this project at any public address; you host your own.

## License

See the repository license file.
