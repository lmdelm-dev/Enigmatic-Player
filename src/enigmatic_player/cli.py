"""Command line interface: launch the TUI or drive one-shot operations.

Examples
--------
    enigmatic                       # launch the TUI
    enigmatic play ~/Music/lofi.mp3 # play a file / folder / youtube url
    enigmatic search "lofi" --provider youtube
    enigmatic config --library ~/Music
    enigmatic status                # print saved state (queue meta)
"""

from __future__ import annotations

import argparse
import shutil
import subprocess
import sys
from pathlib import Path

from . import __version__
from .config import Config
from .core.track import Source
from .providers.local import AUDIO_EXTS
from .providers.manager import ProviderManager

PROVIDER_CHOICES = [s.value for s in Source]  # local, youtube


def _build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(
        prog="enigmatic",
        description="Enigmatic Player — a cute terminal music player.",
    )
    parser.add_argument("--version", action="version", version=f"enigmatic {__version__}")
    sub = parser.add_subparsers(dest="command")

    sub.add_parser("tui", help="Launch the TUI (default when no command is given)")

    play = sub.add_parser("play", help="Play a local file/folder or a URL.")
    play.add_argument("target", help="File path / directory / URL (mp4 webm yt)")
    play.add_argument("--shuffle", action="store_true", help="Shuffle the playlist")

    search = sub.add_parser("search", help="Search a provider.")
    search.add_argument("query")
    search.add_argument("--provider", choices=PROVIDER_CHOICES, default="youtube")
    search.add_argument("--limit", type=int, default=15)

    cfg = sub.add_parser("config", help="Configure libraries and credentials.")
    cfg.add_argument("--library", help="Add a music directory to the local library")
    cfg.add_argument("--provider", help="Set default provider")
    cfg.add_argument("--youtube-quality", choices=["best", "high", "medium", "low"],
                     help="Set YouTube audio quality (best=default, high/medium/low)")

    fmt = sub.add_parser("formats", help="List available YouTube formats for a video/URL.")
    fmt.add_argument("url", help="YouTube URL or video ID")

    sub.add_parser("status", help="Show config summary")
    return parser


def main(argv=None) -> int:
    parser = _build_parser()
    args = parser.parse_args(argv)

    if args.command is None or args.command == "tui":
        return _run_tui()

    if args.command == "play":
        return _cmd_play(args.target, args.shuffle)
    if args.command == "search":
        return _cmd_search(args.query, args.provider, args.limit)
    if args.command == "config":
        return _cmd_config(args)
    if args.command == "formats":
        return _cmd_formats(args.url)
    if args.command == "status":
        return _cmd_status()
    parser.print_help()
    return 0


def _run_tui() -> int:
    from .app import EnigmaticApp

    EnigmaticApp().run()
    return 0


def _cmd_play(target: str, shuffle: bool = False) -> int:
    mpv = shutil.which("mpv")
    if not mpv:
        print("mpv is required for playback. Install it first.", file=sys.stderr)
        return 1

    args = [mpv, "--no-video", "--force-window=no", "--terminal=no"]

    files: list[str] = []
    if "://" in target:
        # A URL is not a filesystem path: Path would collapse its double slash.
        files = [target]
    else:
        path = Path(target).expanduser().resolve()
        if path.is_dir():
            files = sorted(
                str(p) for p in path.rglob("*")
                if p.is_file() and p.suffix.lower() in AUDIO_EXTS
            )
            if not files:
                print(f"No audio files found under {path}", file=sys.stderr)
                return 1
            if shuffle:
                import random

                random.shuffle(files)
        elif path.is_file():
            files = [str(path)]
        else:
            print(f"File not found: {path}", file=sys.stderr)
            return 1

    args.extend(["--", *files])
    print(f"♪ Enigmatic: playing {len(files)} track(s) ♪")
    try:
        return subprocess.call(args)
    except KeyboardInterrupt:
        return 0


def _cmd_search(query: str, provider: str, limit: int) -> int:
    manager = ProviderManager(Config())
    prov = manager.by_source(Source(provider))
    if not prov.available:
        print(f"Provider '{provider}' is not available. Install its extras: "
              "pip install 'enigmatic-player[provider]'", file=sys.stderr)
        return 1
    print(f"Searching {provider} for: {query}\n")
    try:
        tracks = prov.search(query, limit=limit)
    except Exception as exc:  # noqa: BLE001
        print(f"Search failed: {exc}", file=sys.stderr)
        return 1
    for i, t in enumerate(tracks, 1):
        dur = _fmt(t.duration)
        print(f"{i:>2}. {t.title} — {t.artist}  [{dur}]  ({t.provider.value}:{t.uri})")
    print(f"\n{len(tracks)} results.")
    return 0


def _cmd_config(args) -> int:
    cfg = Config()
    if args.library:
        cfg.add_library_dir(args.library)
        cfg.save()
        print(f"Added library dir: {str(Path(args.library).expanduser())}")
    if args.provider:
        cfg.set("default_provider", args.provider)
        cfg.save()
        print(f"Default provider set to {args.provider}")
    if args.youtube_quality:
        cfg.set_youtube_quality(args.youtube_quality)
        print(f"YouTube quality set to {args.youtube_quality}")
    if not any([args.library, args.provider, args.youtube_quality]):
        print("Nothing to do. See `enigmatic config --help`.")
        return 1
    return 0


def _cmd_formats(url: str) -> int:
    """List available YouTube audio formats for a video."""
    # Extract video ID from URL if needed
    import re

    import yt_dlp
    m = re.search(r"(?:v=|youtu\.be/|/embed/|/shorts/)([a-zA-Z0-9_-]{11})", url)
    video_id = m.group(1) if m else url

    ydl = yt_dlp.YoutubeDL({
        "format": "all",
        "quiet": True,
        "no_warnings": True,
        "js_runtimes": {"node": {}},
    })
    try:
        info = ydl.extract_info(f"https://www.youtube.com/watch?v={video_id}", download=False)
    except Exception as exc:
        print(f"Failed to fetch formats: {exc}", file=sys.stderr)
        return 1

    print(f"Available audio formats for {video_id}:")
    print(f"{'ID':>5}  {'Ext':>4}  {'ABR':>6}  {'Codec':>12}  {'Note'}")
    print("-" * 50)
    for f in info.get("formats", []):
        vcodec = f.get('vcodec')
        acodec = f.get('acodec')
        if vcodec == 'none' and acodec and acodec != 'none':
            abr = f.get('abr')
            print(f"{f.get('format_id'):>5}  {f.get('ext'):>4}  {str(abr):>6} kbps  {acodec:>12}  {f.get('format_note', '')}")
    return 0


def _cmd_status() -> int:
    cfg = Config()
    print(f"Library dirs: {', '.join(cfg.library_dirs) or '(none — add with `enigmatic config --library`)'}")
    print(f"Default provider: {cfg.default_provider}")
    print(f"YouTube quality: {cfg.youtube_quality}")
    state = cfg.load_state()
    if state:
        print(f"Last-session queue: {len(state.get('queue', []))} track(s)")
    return 0


def _fmt(seconds: float) -> str:
    if not seconds:
        return "--:--"
    m, s = divmod(int(seconds), 60)
    return f"{m}:{s:02d}"
