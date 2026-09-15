# Enigmatic Player

A lightweight **TUI music player** for your terminal. Play local files or
search YouTube Music, all from a keyboard-first Textual interface — with a
one-shot CLI for quick control.

Built with [Textual](https://textual.textualize.io/) + [mpv](https://mpv.io).

![screenshot](screenshot.svg)

## Features

- 📂 **Local library** — scan folders, read ID3/FLAC/MP4 tags, extract embedded art
- 🎵 **YouTube Music** — search songs and play full-length audio
  (metadata: `ytmusicapi`, streams: `yt-dlp`)
- 🎚 **mpv engine** — gapless playback driven over JSON IPC
  (play/pause/seek/volume), works on Linux / macOS / Windows
- 🗒 **Queue engine** — shuffle, repeat, add/enqueue, session resume
- 🎨 **Album art** — cover art rendered in the terminal, dithered via Pillow
- ⏩ **Playback effects** — speed control (0.5x–2x), reverb, and pitch
  (tempo / nightcore) modes
- ⌨️ **Keyboard-first** TUI with an animated mini-EQ and progress bar
- 🚀 **One-shot CLI** — `enigmatic play "~/Music"`, `enigmatic search "lofi"`

## Requirements

- **Python 3.10+**
- **mpv** — the audio engine (system binary)
- **yt-dlp** (for YouTube) — typically pulled in automatically

Install mpv: `sudo apt install mpv` · `brew install mpv` · `winget install mpv`

## Install

### One-liner (requires Python 3.10+, git, and mpv)

**Linux / macOS:**
```bash
curl -fsSL https://raw.githubusercontent.com/lmdelm-dev/music-player/main/install.sh | bash
```

**Windows (PowerShell):**
```powershell
irm https://raw.githubusercontent.com/lmdelm-dev/music-player/main/install.ps1 | iex
```

**Windows (cmd):**
```cmd
curl -fsSL https://raw.githubusercontent.com/lmdelm-dev/music-player/main/install.cmd | cmd
```

Then launch with:
```bash
epm          # or: enigmatic
```

### Manual (pip)

```bash
pip install "git+https://github.com/lmdelm-dev/music-player.git"
```

Optional extras:
| Extra | Includes |
|---|---|
| `youtube` | `ytmusicapi`, `yt-dlp` |
| `art` | `pillow` (album cover rendering) |
| `dev` | `pytest`, `ruff` |

### Install helpers

Run these from your downloaded or cloned checkout. The helpers install the
`youtube` and `art` extras into the active virtual environment, or create
`.venv` in the checkout if none is active. They require Python 3.10+ and mpv.

Linux/macOS:
```bash
bash install.sh
source .venv/bin/activate  # unless an existing environment was used
```

Windows (PowerShell):
```powershell
.\install.ps1
.\.venv\Scripts\Activate.ps1  # unless an existing environment was used
```

## Usage

### The TUI

```bash
enigmatic          # launch
python -m enigmatic_player
```

| Key | Action |
|---|---|
| `Enter` | play highlighted |
| `p` | play / pause |
| `n` `b` | next / previous |
| `j` `k` | move up / down in list |
| `/` | focus search bar |
| `a` | enqueue highlighted |
| `t` | toggle queue view |
| `c` | clear queue |
| `x` | shuffle on/off |
| `r` | repeat on/off |
| `+` `-` | volume up / down |
| `[` `]` | speed down / up |
| `m` | toggle reverb |
| `y` | toggle pitch mode (tempo / nightcore) |
| `h` | focus playlists / leave playlist view |
| `H` | choose a playlist for the highlighted track |
| `N` | create playlist |
| `R` | rename the open playlist |
| `D` | delete the open playlist |
| `q` | quit |

The sidebar switches sources (Local / YouTube). For YouTube, type a query in the search bar and press Enter.

Press `H` (or a result's heart button) to choose a saved playlist with the
arrow keys and Enter; Escape cancels. Open playlists from the sidebar and
use the remove button to delete individual tracks. Playing a playlist track
loads that playlist into the playback queue.

### One-shot CLI

```bash
enigmatic tui                              # launch the TUI (default)
enigmatic play ~/Music/lofi/               # play a folder
enigmatic play song.mp3                    # play a file
enigmatic play https://youtu.be/...        # play a URL
enigmatic search "lofi" --provider youtube --limit 10
enigmatic formats <video-url>              # list YouTube audio formats
enigmatic config --library ~/Music         # add a music folder
enigmatic status                           # show config summary
```

## YouTube Music setup

Nothing required — `ytmusicapi` works anonymously for search/exists, and
`yt-dlp` resolves stream URLs. A free YT Music account is enough.

## Configuration

- Config: platform config dir (`~/.config/enigmatic-player/config.json` on
  Linux) — credentials stored with `0600` perms.
- Session (last queue) auto-saved to the data dir and restored on launch.

## Development

```bash
pip install -e ".[dev,youtube,art]"
pytest -q          # unit + headless TUI smoke tests
ruff check src tests
```

## Architecture

```
src/enigmatic_player/
  app.py           # Textual App: layout, key bindings, orchestration
  theme.css        # color theme
  cli.py           # one-shot commands (tui/play/search/formats/config/status)
  config.py        # config + session persistence
  core/
    player.py      # mpv subprocess + JSON IPC (socket / named pipe)
    queue.py       # shuffle/repeat logic
    track.py       # unified Track model
  providers/
    local.py       # folder scan + tag/art extraction
    youtube.py     # ytmusicapi + yt-dlp
  ui/
    art.py         # album cover rendering for the terminal
    now_playing.py # art, meta, progress, mini-EQ, transport
    tracklist.py   # reusable list of tracks
```

Audio flows: **UI → provider.resolve_stream(track) → stream URL → mpv**. A
single long-lived mpv process (started with `--idle`) is driven over JSON
IPC: Unix socket on POSIX, named pipe on Windows (ctypes, no pywin32).

## License

MIT