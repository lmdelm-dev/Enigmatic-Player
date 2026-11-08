# Enigmatic Player

Enigmatic Player is a pixel-terminal music player with a Rust desktop edition and a browser edition.

## Web edition

The web edition recreates the original compact terminal layout and core controls:

- 33 color themes from the Rust edition
- Local audio files and user-selected music folders
- Play/pause, previous/next, seek, volume, mute, shuffle and repeat
- Playback speed (0.5x–2.0x)
- Reverb presets and wet amount through Web Audio
- Live spectrum visualizer
- Search/filter, playlists, and keyboard shortcuts
- Optional online search and stream resolution through the included Python backend

### Run locally

For the browser player, serve the repository root with any static web server. For example:

```bash
python -m http.server 8080
```

Open `http://localhost:8080`. You can also publish the root of this repository with GitHub Pages.

Local playback works without a backend. Choose **+ files** or **+ folder** to grant the browser access to your music. Browsers cannot silently scan your disk. Music file handles and object URLs are session-based; playlists and settings are stored in browser local storage.

### Online sources backend

The backend uses FastAPI and yt-dlp. It supports YouTube and SoundCloud search. For Bandcamp, paste a Bandcamp track or album URL into the search field; Bandcamp search itself is not provided by yt-dlp.

Requirements: Python 3.10+ and a current yt-dlp.

```bash
cd backend
python -m venv .venv
# Linux/macOS:
source .venv/bin/activate
# Windows:
# .venv\\Scripts\\activate
pip install -r requirements.txt
uvicorn main:app --host 0.0.0.0 --port 8000
```

Set the backend URL in the web player's **Settings** (for local testing: `http://localhost:8000`). The health endpoint is `/health`.

To deploy the backend, use the included `backend/Dockerfile` or `render.yaml`. The Render free web service may sleep when idle and can have cold-start delays; free hosting availability and limits can change. Set `ALLOWED_ORIGINS` to your published web app origin when deploying publicly. The default `*` is convenient for local testing, not the preferred production setting.

**Important browser limitation:** the backend currently resolves provider media URLs and hands them to the browser. Provider URL expiry, CORS restrictions, browser codec support, rate limits, and provider changes can affect playback. A backend does not make third-party sources permanently reliable. Use only content you have permission to stream.

### Browser vs desktop differences

The Rust edition uses native audio devices, terminal mouse regions, filesystem scanning, and an ffmpeg-backed stream pipeline. A browser cannot select arbitrary system audio outputs or scan folders without user permission. Browser streaming also depends on provider URLs and supported codecs. Those native-only behaviors cannot be reproduced exactly in a static web page.

## Rust desktop edition

The original Rust terminal player remains in `enigmatic-player-rust/`. See its source and Cargo manifest for native requirements and build instructions.

## License

See the repository license file.
