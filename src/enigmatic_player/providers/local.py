"""Local library provider: scans folders, reads tags, finds embedded art."""

from __future__ import annotations

import os
from pathlib import Path
from typing import List, Optional

from mutagen import File as MutagenFile
from mutagen.flac import FLAC
from mutagen.id3 import APIC
from mutagen.mp3 import MP3

from ..config import Config
from ..core.track import Source, Track
from .base import Provider

AUDIO_EXTS = {".mp3", ".m4a", ".flac", ".ogg", ".opus", ".wav", ".aac", ".wma", ".oga"}


class LocalProvider(Provider):
    source = Source.LOCAL
    display_name = "Local"

    def __init__(self, config: Optional[Config] = None) -> None:
        self._config = config or Config()
        self._cache: Optional[List[Track]] = None

    @property
    def available(self) -> bool:
        return True

    # ---- scanning --------------------------------------------------------------
    def _root_dirs(self) -> List[Path]:
        dirs = self._config.library_dirs
        if not dirs:
            default = Path.home() / "Music"
            dirs = [str(default)] if default.exists() else []
        return [Path(d) for d in dirs if Path(d).exists()]

    def _iter_files(self) -> List[Path]:
        files: List[Path] = []
        for root in self._root_dirs():
            for dirpath, _dirnames, filenames in os.walk(root):
                for name in filenames:
                    if Path(name).suffix.lower() in AUDIO_EXTS:
                        files.append(Path(dirpath) / name)
        files.sort(key=lambda p: str(p).lower())
        return files

    def scan(self, force: bool = False) -> List[Track]:
        if self._cache is not None and not force:
            return self._cache
        tracks = [self._track_from_file(p) for p in self._iter_files()]
        self._cache = tracks
        return tracks

    def _track_from_file(self, path: Path) -> Track:
        title, artist, album, art, duration = self._read_tags(path)
        return Track(
            title=title,
            artist=artist,
            album=album,
            duration=duration,
            uri=str(path),
            provider=self.source,
            albumart=art,
        )

    # ---- tags --------------------------------------------------------------------
    @staticmethod
    def _read_tags(path: Path):
        """Return title, artist, album, embedded cover bytes and duration."""
        title = artist = album = ""
        art: Optional[bytes] = None
        duration = 0.0
        try:
            meta = MutagenFile(path)
            if meta is not None:
                title = _tag_text(meta, "title", "TIT2", "\u00a9nam", "Title")
                artist = _tag_text(meta, "artist", "TPE1", "\u00a9ART", "Author")
                album = _tag_text(meta, "album", "TALB", "\u00a9alb", "WM/AlbumTitle")
                duration = float(getattr(meta.info, "length", 0.0) or 0.0)
                art = _extract_cover(meta)
        except Exception:  # noqa: BLE001 - corrupt files shouldn't crash the scan
            pass
        if not title:
            title = path.stem
        return title, artist, album, art, duration

    # ---- Provider API ------------------------------------------------------------
    def search(self, query: str, limit: int = 30) -> List[Track]:
        q = query.lower()
        tracks = self.scan()
        matched = [t for t in tracks if q in f"{t.title} {t.artist} {t.album}".lower()]
        return matched[:limit]

    def browse(self, path: str = "") -> List[Track]:
        return self.scan()

    def resolve_stream(self, track: Track) -> Optional[str]:
        p = Path(track.uri)
        return str(p) if p.exists() else None


def _tag_text(meta, *keys: str) -> str:
    for key in keys:
        value = meta.get(key)
        if value:
            value = getattr(value, "text", value)
            if isinstance(value, (list, tuple)):
                return ", ".join(str(part) for part in value)
            return str(value)
    return ""


def _extract_cover(meta) -> Optional[bytes]:
    """Pull embedded cover art (picture) as raw bytes if available."""
    try:
        if isinstance(meta, FLAC):
            pics = meta.pictures
            if pics:
                return bytes(pics[0].data)
        if isinstance(meta, MP3):
            if meta.tags:
                for tag in meta.tags.values():
                    if isinstance(tag, APIC):
                        return bytes(tag.data)
        elif "covr" in meta:  # MP4 / M4A
            return bytes(meta["covr"][0])
    except Exception:  # noqa: BLE001
        return None
    return None
