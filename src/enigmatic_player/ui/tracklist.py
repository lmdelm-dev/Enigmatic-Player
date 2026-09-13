"""Selectable list of tracks (reused by search results and queue)."""

from __future__ import annotations

from typing import List, Optional

from textual.app import ComposeResult
from textual.containers import Horizontal, Vertical
from textual.widgets import Button, Label, ListItem, ListView

from ..core.track import Source, Track, fmt_time

PROVIDER_ICON = {"local": "◎", "youtube": "▶"}
GB_INK = "rgb(43,255,150)"
GB_MID = "rgb(28,125,78)"
GB_RED = "rgb(255,80,80)"


class TrackListItem(ListItem):
    def __init__(
        self,
        track: Track,
        number: int = 0,
        show_heart: bool = False,
        on_heart: Optional[callable] = None,
        on_download: Optional[callable] = None,
        **kwargs,
    ) -> None:
        self.track = track
        self.number = number
        self.show_heart = show_heart
        self.on_heart = on_heart
        self.on_download = on_download
        super().__init__(**kwargs)

    def compose(self) -> ComposeResult:
        icon = PROVIDER_ICON.get(self.track.provider.value, "•")
        duration = fmt_time(self.track.duration) if self.track.duration else "--:--"
        # Truncate long fields for terminal width
        title = self.track.title[:48]
        artist = (self.track.artist or "").strip()[:28]
        artist_part = f" — [{GB_MID}]{artist}[/]" if artist else ""
        line = (
            f"[{GB_MID}]{icon}[/] "
            f"[{GB_MID}]{self.number:02d}[/] "
            f"[bold {GB_INK}]{title}[/]"
            f"{artist_part} "
            f"[{GB_INK}]{duration}[/]"
        )
        buttons = []
        if self.on_download:
            buttons.append(Button("DL", id="btn-download", classes="download-btn"))
        if self.show_heart:
            buttons.append(Button("♡", id="btn-heart", classes="heart-btn"))
        yield Horizontal(
            Label(line, classes="track-main"),
            *buttons,
            classes="track-row",
        )

    def on_button_pressed(self, event: Button.Pressed) -> None:
        if event.button.id == "btn-heart" and self.on_heart:
            self.on_heart(self.track)
        elif event.button.id == "btn-download" and self.on_download:
            self.on_download(self.track)


class PlaylistListItem(ListItem):
    """Item in the playlist sidebar list."""
    def __init__(self, playlist: dict, index: int, **kwargs) -> None:
        self.playlist = playlist
        self.index = index
        super().__init__(**kwargs)

    def compose(self) -> ComposeResult:
        name = self.playlist.get("name", "Untitled")[:20]
        count = len(self.playlist.get("tracks", []))
        # pill-style count
        yield Label(f"▸ [bold {GB_INK}]{name}[/]  [{GB_MID}]· {count} tracks[/]")


class PlaylistTrackItem(ListItem):
    """Track inside a playlist view (with remove button)."""
    def __init__(self, track: Track, index: int, on_remove: callable, **kwargs) -> None:
        self.track = track
        self.index = index
        self.on_remove = on_remove
        super().__init__(**kwargs)

    def compose(self) -> ComposeResult:
        duration = fmt_time(self.track.duration) if self.track.duration else "--:--"
        title = self.track.title[:42]
        artist = (self.track.artist or "")[:24]
        line = (
            f"[{GB_MID}]{self.index + 1:02d}[/] "
            f"[bold {GB_INK}]{title}[/]  "
            f"[{GB_MID}]{artist}[/]  "
            f"[{GB_INK}]{duration}[/]"
        )
        yield Horizontal(
            Label(line, classes="track-main"),
            Button("✕", id="btn-remove", classes="remove-btn"),
            classes="track-row",
        )

    def on_button_pressed(self, event: Button.Pressed) -> None:
        if event.button.id == "btn-remove" and self.on_remove:
            self.on_remove(self.index)


class TrackList(Vertical):
    """Wraps a :class:`ListView` and exposes track-level helpers."""

    def __init__(self, **kwargs) -> None:
        super().__init__(**kwargs)
        self._list: Optional[ListView] = None

    def compose(self) -> ComposeResult:
        lv_id = f"{self.id}-items" if self.id else None
        self._list = ListView(id=lv_id)
        yield self._list

    # ---- population ------------------------------------------------------------
    def set_tracks(
        self,
        tracks: Optional[List[Track]] = None,
        show_heart: bool = False,
        on_heart: Optional[callable] = None,
        on_download: Optional[callable] = None,
    ) -> None:
        if self._list is None:
            return
        self._list.clear()
        if tracks:
            for i, t in enumerate(tracks, start=1):
                self._list.append(
                    TrackListItem(
                        t, number=i, show_heart=show_heart,
                        on_heart=on_heart, on_download=on_download,
                    )
                )
        self.call_after_refresh(self._select_first)

    def _select_first(self) -> None:
        if self._list is not None:
            self._list.index = 0 if self._list.children else None

    def set_playlist_tracks(
        self,
        playlist: dict,
        on_remove: callable,
    ) -> None:
        """Populate with playlist tracks (PlaylistTrackItem with remove button)."""
        if self._list is None:
            return
        self._list.clear()
        tracks = playlist.get("tracks", [])
        for i, t_data in enumerate(tracks):
            track = Track(
                title=t_data.get("title", ""),
                artist=t_data.get("artist", ""),
                album=t_data.get("album", ""),
                duration=float(t_data.get("duration") or 0),
                uri=t_data.get("uri", ""),
                provider=Source(t_data.get("provider") or "local"),
                track_id=t_data.get("track_id", ""),
                cover_uri=t_data.get("cover_uri"),
            )
            self._list.append(PlaylistTrackItem(track, index=i, on_remove=on_remove))
        self.call_after_refresh(self._select_first)

    def append_track(self, track: Track) -> None:
        if self._list is None:
            return
        self._list.append(TrackListItem(track, number=self._list.child_count + 1))

    def clear(self) -> None:
        if self._list:
            self._list.clear()

    # ---- introspection ------------------------------------------------------------
    @property
    def items(self) -> List[Track]:
        if self._list is None:
            return []
        return [
            c.track for c in self._list.children
            if isinstance(c, (TrackListItem, PlaylistTrackItem))
        ]

    @property
    def index(self) -> Optional[int]:
        return self._list.index if self._list else None

    @property
    def selected(self) -> Optional[Track]:
        if self._list is None:
            return None
        idx = self._list.index
        if idx is None or not 0 <= idx < len(self._list.children):
            return None
        child = self._list.children[idx]
        return child.track if isinstance(child, (TrackListItem, PlaylistTrackItem)) else None

    # ---- focus / cursor -----------------------------------------------------------
    def focus_filter(self) -> None:
        if self._list:
            self._list.focus()

    def cursor_down(self) -> None:
        if self._list:
            self._list.action_cursor_down()

    def cursor_up(self) -> None:
        if self._list:
            self._list.action_cursor_up()


class PlaylistSidebar(Vertical):
    """Sidebar showing saved playlists."""
    def __init__(self, on_select: callable, on_new: callable, **kwargs) -> None:
        super().__init__(**kwargs)
        self.on_select = on_select
        self.on_new = on_new
        self._list: Optional[ListView] = None

    def compose(self) -> ComposeResult:
        yield Label("PLAYLISTS", classes="source-title")
        self._list = ListView(id="playlist-list")
        yield self._list
        yield Button("+ New Playlist", id="btn-new-playlist", classes="side")

    def on_mount(self) -> None:
        self._list.focus()

    def set_playlists(self, playlists: List[dict]) -> None:
        if self._list is None:
            return
        self._list.clear()
        for i, pl in enumerate(playlists):
            self._list.append(PlaylistListItem(pl, index=i))

    @property
    def selected_index(self) -> Optional[int]:
        if self._list is None:
            return None
        idx = self._list.index
        if idx is None:
            return None
        child = self._list.children[idx]
        return child.index if isinstance(child, PlaylistListItem) else None

    def on_list_view_selected(self, event: ListView.Selected) -> None:
        if self.on_select:
            idx = self.selected_index
            if idx is not None:
                self.on_select(idx)

    def on_button_pressed(self, event: Button.Pressed) -> None:
        if event.button.id == "btn-new-playlist" and self.on_new:
            self.on_new()
