"""Locate (and on Windows, auto-install) the audio engines mpv and ffmpeg.

The player prefers whatever is already available on ``PATH``. If missing, we
fall back to engines bundled into the app data dir (``<data-dir>/engines``).
On Windows the official static builds are downloaded automatically on first
run, so the player works on a clean machine with nothing but Python installed.

Linux/macOS are not auto-installed (no official static builds); the app just
prints the one command needed for the detected package manager.
"""

from __future__ import annotations

import logging
import shutil
import sys
import zipfile
from pathlib import Path
from typing import Dict, Optional

log = logging.getLogger(__name__)

_MPV_GITHUB_API = "https://api.github.com/repos/mpv-player/mpv/releases/latest"
_FFMPEG_ESSENTIALS = "https://www.gyan.dev/ffmpeg/builds/ffmpeg-release-essentials.zip"

_MPV_BIN = "mpv.exe" if sys.platform == "win32" else "mpv"
_FFMPEG_BIN = "ffmpeg.exe" if sys.platform == "win32" else "ffmpeg"


def engines_dir() -> Path:
    """Directory where bundled engines live (mpv.exe/DLLs, ffmpeg.exe)."""
    from platformdirs import user_data_dir

    return Path(user_data_dir("enigmatic-player")) / "engines"


def mpv_path() -> Optional[str]:
    """Full path to an mpv executable: PATH first, then the bundled copy.

    On Windows, prefer the real ``mpv.exe`` over the ``mpv.com`` console
    launcher — driving JSON IPC through the .com wrapper is unreliable.
    """
    # Prefer a real mpv.exe over the mpv.com console launcher on Windows —
    # driving JSON IPC through the .com wrapper is unreliable.
    names = ("mpv.exe", "mpv") if sys.platform == "win32" else ("mpv",)
    for name in names:
        found = shutil.which(name)
        if found and Path(found).name.endswith("mpv.exe"):
            return found
    for candidate in engines_dir().rglob(_MPV_BIN):
        return str(candidate)
    return None


def ffmpeg_path() -> Optional[str]:
    """Full path to an ffmpeg executable: PATH first, then the bundled copy."""
    found = shutil.which("ffmpeg")
    if found:
        return found
    for candidate in engines_dir().rglob(_FFMPEG_BIN):
        return str(candidate)
    return None


def ensure_engines() -> Dict[str, Optional[str]]:
    """Make mpv + ffmpeg available, auto-downloading on Windows if needed.

    Blocking and chatty (prints download progress), so call *before* the TUI
    starts. Returns a ``{"mpv": ..., "ffmpeg": ...}`` mapping of resolved
    paths â€” either may stay ``None``.
    """
    mpv = mpv_path()
    ffmpeg = ffmpeg_path()

    if sys.platform == "win32" and (mpv is None or ffmpeg is None):
        print("Enigmatic Player: preparing audio engines (first run â€” one-time download)...")
    if mpv is None and sys.platform == "win32":
        mpv = _install_mpv_windows()
    if ffmpeg is None and sys.platform == "win32":
        ffmpeg = _install_ffmpeg_windows()

    if mpv is None:
        print(mpv_hint())
    if ffmpeg is None:
        print(ffmpeg_hint())
    return {"mpv": mpv, "ffmpeg": ffmpeg}


def mpv_hint() -> str:
    if sys.platform == "win32":
        return (
            "mpv could not be installed automatically. Install it with:\n"
            "    winget install -e --id mpv-player.mpv-CI.MSVC"
        )
    return (
        "mpv is required for playback. Install it:\n"
        "    " + _package_manager_cmd("mpv")
    )


def ffmpeg_hint() -> str:
    if sys.platform == "win32":
        return (
            "ffmpeg could not be installed automatically. Install it with:\n"
            "    winget install -e --id Gyan.FFmpeg"
        )
    return (
        "ffmpeg is required for YouTube MP3 downloads. Install it:\n"
        "    " + _package_manager_cmd("ffmpeg")
    )


def _package_manager_cmd(pkg: str) -> str:
    for manager, cmd in (
        ("apt-get", f"sudo apt-get install -y {pkg}"),
        ("dnf", f"sudo dnf install -y {pkg}"),
        ("pacman", f"sudo pacman -S --noconfirm {pkg}"),
        ("zypper", f"sudo zypper install -y {pkg}"),
        ("apk", f"sudo apk add {pkg}"),
    ):
        if shutil.which(manager):
            return cmd
    return f"brew install {pkg}" if sys.platform == "darwin" else f"sudo apt-get install -y {pkg}"


def _fetch(url: str, dest: Path) -> None:
    """Download ``url`` to ``dest`` with a light progress readout."""
    import requests

    dest.parent.mkdir(parents=True, exist_ok=True)
    print(f"  downloading {url.split('/')[-1]} ...", flush=True)
    with requests.get(url, stream=True, timeout=120) as response:
        response.raise_for_status()
        total = int(response.headers.get("content-length") or 0)
        last_pct = -1
        done = 0
        with open(dest, "wb") as fh:
            for chunk in response.iter_content(1 << 16):
                fh.write(chunk)
                done += len(chunk)
                if total:
                    pct = done * 100 // total
                    if pct >= last_pct + 10:
                        last_pct = pct
                        print(f"  ...{pct}%", end="\r", flush=True)
        print("  done.", flush=True)


def _install_mpv_windows() -> Optional[str]:
    """Download the official mpv MSVC build and unpack it into engines/."""
    exe = engines_dir() / "mpv" / "mpv.exe"
    if exe.exists():
        return str(exe)
    try:
        import requests

        release = requests.get(_MPV_GITHUB_API, timeout=30).json()
        asset = next(
            a for a in release.get("assets", [])
            if a.get("name", "").endswith("x86_64-pc-windows-msvc.zip")
        )
        archive = engines_dir() / "mpv-win64.zip"
        _fetch(asset["browser_download_url"], archive)
        with zipfile.ZipFile(archive) as zipf:
            zipf.extractall(engines_dir() / "mpv")
        archive.unlink(missing_ok=True)
        return str(exe) if exe.exists() else None
    except Exception as exc:  # noqa: BLE001 - best effort
        log.debug("mpv auto-install failed: %s", exc)
        return None


def _install_ffmpeg_windows() -> Optional[str]:
    """Download the gyan.dev static ffmpeg build and unpack it into engines/."""
    exe = engines_dir() / "ffmpeg" / "bin" / "ffmpeg.exe"
    if exe.exists():
        return str(exe)
    try:
        archive = engines_dir() / "ffmpeg.zip"
        _fetch(_FFMPEG_ESSENTIALS, archive)
        tmp = engines_dir() / "ffmpeg_tmp"
        if tmp.exists():
            shutil.rmtree(tmp)
        with zipfile.ZipFile(archive) as zipf:
            zipf.extractall(tmp)
        archive.unlink(missing_ok=True)
        found = next(tmp.rglob("ffmpeg.exe"), None)
        if found:
            exe.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(found, exe)
            shutil.rmtree(tmp, ignore_errors=True)
            return str(exe)
        return None
    except Exception as exc:  # noqa: BLE001 - best effort
        log.debug("ffmpeg auto-install failed: %s", exc)
        return None

