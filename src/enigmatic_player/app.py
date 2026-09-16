"""Enigmatic Player — the main Textual app."""

from __future__ import annotations

import asyncio
from typing import List, Optional

from textual.app import App, ComposeResult, on
from textual.binding import Binding
from textual.containers import Horizontal, Vertical
from textual.screen import ModalScreen
from textual.widgets import Button, Footer, Header, Input, Label, ListView
from textual.worker import Worker, WorkerState

from .config import Config
from .core.player import MpvError, MpvPlayer
from .core.queue import Queue
from .core.track import Source, Track
from .providers.manager import ProviderManager
from .ui.now_playing import NowPlaying
from .ui.tracklist import (
    PlaylistListItem,
    PlaylistSidebar,
    PlaylistTrackItem,
    TrackList,
    TrackListItem,
)

ALL_SOURCES = [Source.LOCAL, Source.YOUTUBE]


class PlaylistInputScreen(ModalScreen[str]):
    """Modal input for playlist name (create/rename)."""
    def __init__(self, title: str, default: str = "") -> None:
        super().__init__()
        self._title = title
        self._default = default

    def compose(self) -> ComposeResult:
        yield Vertical(
            Label(self._title, classes="dialog-title"),
            Input(placeholder="e.g. Late Night Lofi", value=self._default, id="playlist-input"),
            Horizontal(
                Button("✓  Create", id="btn-create", variant="success"),
                Button("✕  Cancel", id="btn-cancel"),
                classes="dialog-buttons",
            ),
            classes="dialog",
        )

    def on_mount(self) -> None:
        self.call_after_refresh(self.query_one("#playlist-input", Input).focus)

    @on(Input.Submitted, "#playlist-input")
    def _submit(self, event: Input.Submitted) -> None:
        self.dismiss(event.value.strip())

    @on(Button.Pressed, "#btn-cancel")
    def _cancel(self) -> None:
        self.dismiss("")

    @on(Button.Pressed, "#btn-create")
    def _create(self) -> None:
        self.dismiss(self.query_one("#playlist-input", Input).value.strip())


class PlaylistPickerScreen(ModalScreen[Optional[int]]):
    """Choose a destination without changing playlists on cancellation."""

    BINDINGS = [Binding("escape", "cancel", "Cancel")]

    def __init__(self, playlists: List[dict]) -> None:
        super().__init__()
        self._playlists = playlists

    def compose(self) -> ComposeResult:
        yield Vertical(
            Label("Add to playlist", classes="dialog-title"),
            ListView(
                *(PlaylistListItem(pl, index=i) for i, pl in enumerate(self._playlists)),
                id="playlist-picker",
            ),
            Button("Cancel", id="picker-cancel"),
            classes="dialog",
        )

    def on_mount(self) -> None:
        self.query_one("#playlist-picker", ListView).focus()

    @on(ListView.Selected, "#playlist-picker")
    def _select(self, event: ListView.Selected) -> None:
        event.stop()
        if isinstance(event.item, PlaylistListItem):
            self.dismiss(event.item.index)

    @on(Button.Pressed, "#picker-cancel")
    def action_cancel(self) -> None:
        self.dismiss(None)


class EnigmaticApp(App):
    CSS_PATH = "theme.css"
    TITLE = "Enigmatic Player"
    BINDINGS = [
        Binding("q", "quit", "Quit"),
        Binding("p", "play_pause", "Play/Pause"),
        Binding("n", "next", "Next"),
        Binding("b", "prev", "Prev"),
        Binding("/", "focus_search", "Search"),
        Binding("j", "move_down", "Down", show=False),
        Binding("k", "move_up", "Up", show=False),
        Binding("a", "add_to_queue", "Enqueue"),
        Binding("c", "clear_queue", "Clear queue"),
        Binding("t", "toggle_queue", "Queue"),
        Binding("x", "shuffle", "Shuffle"),
        Binding("r", "repeat", "Repeat"),
        Binding("+", "vol_up", "Vol +"),
        Binding("-", "vol_down", "Vol -"),
        Binding("[", "speed_down", "Speed -"),
        Binding("]", "speed_up", "Speed +"),
        Binding("m", "toggle_reverb", "Reverb"),
        Binding("y", "toggle_pitch", "Pitch"),
        Binding("h", "toggle_playlist_mode", "Playlists"),
        Binding("H", "heart_track", "♥ Add to playlist"),
        Binding("N", "new_playlist", "New playlist"),
        Binding("R", "rename_playlist", "Rename playlist"),
        Binding("D", "delete_playlist", "Delete playlist"),
    ]

    def __init__(
        self,
        config: Optional[Config] = None,
        manager: Optional[ProviderManager] = None,
    ) -> None:
        super().__init__()
        self._config = config or Config()
        self._manager = manager or ProviderManager(self._config)
        self.queue = Queue()
        self.queue.on_change = self._refresh_queue
        self.player: Optional[MpvPlayer] = None
        self._source = Source.LOCAL
        self._queue_mode = False
        self._playlist_mode = False
        self._current_playlist_index: Optional[int] = None
        self._current_track: Optional[Track] = None
        self._known_length = 0.0
        self._meta_ticks = 0

    # ------------------------------------------------------------------ compose
    def compose(self) -> ComposeResult:
        yield Header(show_clock=True)
        with Horizontal(id="body"):
            with Vertical(id="sidebar"):
                yield Label("SOURCES", classes="source-title")
                yield Button("▣  Local", id="src-local", classes="side active")
                yield Button("▶  YouTube", id="src-youtube", classes="side")
                yield PlaylistSidebar(
                    id="playlist-sidebar",
                    on_select=self._on_playlist_selected,
                    on_new=self.action_new_playlist,
                )
                yield Label("VIEW", classes="source-title")
                yield Button("▤  Queue", id="btn-queue", classes="side")
                yield Label(
                    "[bold #ebdbb2]▶ CONTROLS[/]\n"
                    "[bold]p[/] play/pause  [bold]n[/]/[bold]b[/] next/prev\n"
                    "[bold]Enter[/] play  [bold]a[/] enqueue  [bold]c[/] clear\n"
                    "[bold]j[/]/[bold]k[/] nav  [bold]/[/] search\n"
                    "[bold #ebdbb2]▶ QUEUE[/]\n"
                    "[bold]t[/] queue  [bold]x[/] shuffle  [bold]r[/] repeat\n"
                    "[bold #ebdbb2]▶ PLAYLISTS[/]\n"
                    "[bold]h[/] browse  [bold]H[/] ♥ add  [bold]N[/] new\n"
                    "[bold]R[/] rename  [bold]D[/] delete\n"
                    "[bold #ebdbb2]▶ FX[/]\n"
                    "[bold]←→[/] adjust  [bold]m[/] reverb  [bold]y[/] pitch\n"
                    "[bold]+[/]/[bold]-[/] vol  [bold]q[/] quit",
                    classes="helpbox",
                )
            with Vertical(id="main"):
                yield Input(placeholder="⌕  search local library, YouTube…", id="searchbar")
                with Vertical(id="listhost"):
                    yield TrackList(id="list-view")
                    yield TrackList(id="queue-view")
        yield NowPlaying(id="nowplaying")
        yield Footer()

    # ------------------------------------------------------------------ mount
    def on_mount(self) -> None:
        try:
            self.player = MpvPlayer()
            self.player.start()
        except MpvError as exc:
            self.notify(str(exc), severity="error", timeout=8)
            self.player = None
        self.set_interval(0.1, self._poll)
        self._queue_refreshed = False
        self._restore_session()
        self._refresh_playlist_sidebar()
        asyncio.create_task(self._load_local())

    async def _load_local(self) -> None:
        self.notify("Scanning local library…", timeout=2)
        tracks: List[Track] = await asyncio.to_thread(self._manager.local.scan)
        if self._source is not Source.LOCAL or self._playlist_mode or not self.is_mounted:
            return
        self.query_one("#list-view", TrackList).set_tracks(tracks)
        if len(tracks) == 0:
            self.notify(
                "No music found. Add folders with: `enigmatic config --library /path/to/Music`",
                severity="warning",
                timeout=6,
            )
        elif len(tracks) > 500:
            self.notify(f"Loaded {len(tracks)} local tracks", timeout=2)

    # ---- playlist helpers ------------------------------------------------------
    def _refresh_playlist_sidebar(self) -> None:
        sidebar = self.query_one("#playlist-sidebar", PlaylistSidebar)
        sidebar.set_playlists(self._config.playlists)

    def _enter_playlist_mode(self, playlist_index: int) -> None:
        """Switch to viewing a specific playlist."""
        self._playlist_mode = True
        self._current_playlist_index = playlist_index
        playlist = self._config.playlists[playlist_index]
        # Show playlist tracks in list-view with remove buttons
        list_view = self.query_one("#list-view", TrackList)
        list_view.set_playlist_tracks(playlist, on_remove=self._remove_track_from_current_playlist)
        if self._queue_mode:
            self.action_toggle_queue()
        list_view.focus_filter()
        self._refresh_playlist_sidebar()
        self.notify(f"Playlist: {playlist['name']}", timeout=2)

    def _exit_playlist_mode(self) -> None:
        """Exit playlist view back to normal source view."""
        if self._playlist_mode:
            self._playlist_mode = False
            self._current_playlist_index = None
            # Restore the current source view
            if self._source is Source.LOCAL:
                asyncio.create_task(self._load_local())
            else:
                self.query_one("#list-view", TrackList).clear()
            self._refresh_playlist_sidebar()

    def _add_track_to_playlist(self, track: Track) -> None:
        """Show playlist picker and add track to selected playlist."""
        playlists = self._config.playlists
        if not playlists:
            self.notify("No playlists yet. Press N to create one.", timeout=3)
            return
        def _chosen(index: Optional[int]) -> None:
            if index is None:
                return
            if self._config.add_track_to_playlist(index, self._track_to_dict(track)):
                self.notify(f"Added to '{playlists[index]['name']}'", timeout=2)
                self._refresh_playlist_sidebar()
                if self._playlist_mode and self._current_playlist_index == index:
                    self.query_one("#list-view", TrackList).set_playlist_tracks(
                        self._config.playlists[index],
                        on_remove=self._remove_track_from_current_playlist,
                    )

        self.push_screen(PlaylistPickerScreen(playlists), _chosen)

    def _remove_track_from_current_playlist(self, track_index: int) -> None:
        if self._current_playlist_index is not None:
            self._config.remove_track_from_playlist(self._current_playlist_index, track_index)
            # Refresh the view
            playlist = self._config.playlists[self._current_playlist_index]
            list_view = self.query_one("#list-view", TrackList)
            list_view.set_playlist_tracks(playlist, on_remove=self._remove_track_from_current_playlist)
            self._refresh_playlist_sidebar()

    # ------------------------------------------------------------------ helpers
    def _active_list(self) -> Optional[TrackList]:
        return self.query_one("#queue-view" if self._queue_mode else "#list-view", TrackList)

    def _refresh_queue(self) -> None:
        try:
            self.query_one("#queue-view", TrackList).set_tracks(self.queue.items)
        except Exception:  # noqa: BLE001 - not mounted yet is fine
            pass

    def _queue_from_active_list(self) -> None:
        lv = self._active_list()
        if lv is None:
            return
        tracks = lv.items
        if not tracks:
            self.notify("Nothing in this view yet", timeout=1)
            return
        idx = lv.index or 0
        self.queue.set_items(tracks, start=idx)

    async def play_track(self, track: Track) -> None:
        if not self.player:
            self.notify("mpv is unavailable — can't play audio.", severity="error")
            return
        prov = self._manager.by_source(track.provider)
        if not prov.available:
            self.notify(f"{track.provider.value} provider not available", severity="warning")
            return
        url, headers = await asyncio.to_thread(prov.resolve_stream_with_headers, track)
        if not url:
            self.notify("No playable stream available for this track.", severity="error", timeout=3)
            return
        self.player.load(url, http_headers=headers)
        self.player.play()
        self._current_track = track
        self._known_length = track.duration or 0.0
        self.query_one(NowPlaying).set_track(track, paused=False)

    # ------------------------------------------------------------------ download
    def _download_track(self, track: Track) -> None:
        """Download a YouTube track as MP3 to ~/Downloads/EPM/."""
        import os
        import platformdirs

        downloads = platformdirs.user_downloads_dir()
        epm_dir = os.path.join(downloads, "EPM")
        os.makedirs(epm_dir, exist_ok=True)

        safe_title = "".join(c if c.isalnum() or c in " -_" else "_" for c in track.title)[:80]
        artist = (track.artist or "Unknown").replace("/", "-")
        filename = f"{artist} - {safe_title}.mp3"
        outpath = os.path.join(epm_dir, filename)

        if os.path.exists(outpath):
            self.notify(f"Already downloaded: {filename}", timeout=3)
            return

        self.notify(f"Downloading: {track.title}...", timeout=5)

        def _do_download() -> Optional[str]:
            import shutil

            if not shutil.which("ffmpeg"):
                return (
                    "ffmpeg not found — required to convert audio to MP3.\n"
                    "Install it:  sudo apt install ffmpeg  /  brew install ffmpeg  /  winget install ffmpeg"
                )
            try:
                import yt_dlp
                ydl_opts = {
                    "format": "bestaudio/best",
                    "outtmpl": outpath.replace(".mp3", ".%(ext)s"),
                    "postprocessors": [{
                        "key": "FFmpegExtractAudio",
                        "preferredcodec": "mp3",
                        "preferredquality": "192",
                    }],
                    "quiet": True,
                    "no_warnings": True,
                }
                url = f"https://www.youtube.com/watch?v={track.uri}"
                with yt_dlp.YoutubeDL(ydl_opts) as ydl:
                    ydl.download([url])
                return None
            except Exception as exc:  # noqa: BLE001
                return str(exc)

        self.run_worker(
            _do_download, thread=True, group="download",
            description=filename, exit_on_error=False,
        )

    def on_worker_state_changed(self, event: Worker.StateChanged) -> None:
        """Report download completion back on the UI thread."""
        if event.worker.group != "download":
            return
        if event.state == WorkerState.ERROR:
            error = getattr(event.worker, "error", None)
            self.notify(f"Download failed: {error}", severity="error", timeout=5)
            return
        if event.state != WorkerState.SUCCESS:
            return
        error = event.worker.result
        if error:
            self.notify(f"Download failed: {error}", severity="error", timeout=5)
        else:
            self.notify(f"Saved: {event.worker.description}", timeout=3)

    # ------------------------------------------------------------------ polling
    def _poll(self) -> None:
        if not (self.player and self.player.running):
            return
        if not self.is_mounted:
            return
        try:
            nps = self.query_one(NowPlaying)
        except Exception:  # noqa: BLE001 - NowPlaying may not be mounted yet
            return
        for ev in self.player.drain():
            if ev.get("event") == "end-file" and ev.get("reason") == "eof":
                self._advance()
        st = self.player.state
        if self._current_track:
            if self._known_length <= 0:
                if st.get("duration"):
                    self._known_length = float(st["duration"])
                elif self._meta_ticks % 5 == 0:
                    self.player.refresh_meta()
                self._meta_ticks += 1
        try:
            nps.played = float(st.get("time_pos") or 0.0)
            nps.paused = bool(st.get("paused", True))
            if self._known_length:
                nps.length = self._known_length
            nps.sync_volume(int(st.get("volume", 100) or 100))
            nps.sync_speed(float(st.get("speed", 1.0) or 1.0))
            pitch_mode = str(st.get("pitch_mode", "nightcore"))
            nps.sync_pitch_mode(pitch_mode)
            nps.sync_fx(
                float(st.get("speed", 1.0) or 1.0),
                int(st.get("reverb", 0) or 0),
                pitch_mode,
            )
            nps.tick_eq()
            nps.tick_progress_head()
        except Exception:  # noqa: BLE001 - widgets can be re-rendering mid-poll
            pass

    def _advance(self) -> None:
        nxt = self.queue.next()
        if nxt:
            self.notify(f"{nxt.title}", timeout=1)
            asyncio.create_task(self.play_track(nxt))
        else:
            self._current_track = None
            self.query_one(NowPlaying).set_track(None)

    def _skip_to(self, direction: int) -> None:
        if not self.queue.length:
            return
        adv = self.queue.next() if direction > 0 else self.queue.prev()
        if adv:
            self.notify(f"{adv.title}", timeout=1)
            asyncio.create_task(self.play_track(adv))

    # ------------------------------------------------------------------ actions
    def action_play_pause(self) -> None:
        if not self.player:
            return
        try:
            self.player.toggle()
        except MpvError as e:
            self.notify(f"Playback error: {e}", severity="error")

    def action_next(self) -> None:
        try:
            self._skip_to(1)
        except MpvError as e:
            self.notify(f"Playback error: {e}", severity="error")

    def action_prev(self) -> None:
        try:
            self._skip_to(-1)
        except MpvError as e:
            self.notify(f"Playback error: {e}", severity="error")

    def action_focus_search(self) -> None:
        if isinstance(self.focused, Input):
            return
        self.query_one(Input).focus()

    def action_move_down(self) -> None:
        lv = self._active_list()
        if lv:
            lv.cursor_down()

    def action_move_up(self) -> None:
        lv = self._active_list()
        if lv:
            lv.cursor_up()

    def action_add_to_queue(self) -> None:
        lv = self._active_list()
        if not lv or lv.selected is None:
            return
        self.queue.add(lv.selected)
        self.notify(f"Enqueued {lv.selected.title}", timeout=1)

    def action_clear_queue(self) -> None:
        self.queue.clear()
        self.notify("Queue cleared", timeout=1)

    def action_toggle_queue(self) -> None:
        self._queue_mode = not self._queue_mode
        lv = self.query_one("#list-view", TrackList)
        qv = self.query_one("#queue-view", TrackList)
        btn = self.query_one("#btn-queue", Button)
        if self._queue_mode:
            qv.add_class("visible")
            lv.add_class("invisible")
            btn.label = "✕  Close"
            qv.focus_filter()
        else:
            qv.remove_class("visible")
            lv.remove_class("invisible")
            btn.label = "▤  Queue"

    def action_shuffle(self) -> None:
        on = self.queue.toggle_shuffle()
        self.query_one("#btn-shuffle", Button).set_class(on, "toggled")
        self.notify(f"shuffle {'on' if on else 'off'}", timeout=1)

    def action_repeat(self) -> None:
        on = self.queue.toggle_repeat()
        self.query_one("#btn-repeat", Button).set_class(on, "toggled")
        self.notify(f"repeat {'on' if on else 'off'}", timeout=1)

    def _bump_volume(self, delta: int) -> None:
        if not self.player:
            return
        try:
            self.player.set_volume(self.player.volume() + delta)
            self.notify(f"volume: {self.player.volume()}", timeout=1)
        except MpvError as e:
            self.notify(f"Volume error: {e}", severity="error")

    def action_vol_up(self) -> None:
        self._bump_volume(5)

    def action_vol_down(self) -> None:
        self._bump_volume(-5)

    def action_speed_down(self) -> None:
        if not self.player:
            return
        try:
            s = self.player.adjust_speed(-0.05)
            self.notify(f"speed {s:.2f}x", timeout=1)
        except MpvError as e:
            self.notify(f"Speed error: {e}", severity="error")

    def action_speed_up(self) -> None:
        if not self.player:
            return
        try:
            s = self.player.adjust_speed(0.05)
            self.notify(f"speed {s:.2f}x", timeout=1)
        except MpvError as e:
            self.notify(f"Speed error: {e}", severity="error")

    def action_toggle_reverb(self) -> None:
        if not self.player:
            return
        try:
            on = int(self.player.state.get("reverb", 0)) == 0
            self.player.set_reverb(80 if on else 0)
            self.notify(f"reverb {'on' if on else 'off'}", timeout=1)
        except MpvError as e:
            self.notify(f"Reverb error: {e}", severity="error")

    def action_toggle_pitch(self) -> None:
        if not self.player:
            return
        try:
            mode = "nightcore" if self.player.state.get("pitch_mode") == "tempo" else "tempo"
            self.player.set_pitch_mode(mode)
            self.notify(f"pitch: {mode}", timeout=1)
        except MpvError as e:
            self.notify(f"Pitch error: {e}", severity="error")

    # ------------------------------------------------------------------ session
    def _restore_session(self) -> None:
        state = self._config.load_state()
        raw_queue = state.get("queue") or []
        if not isinstance(raw_queue, list) or not raw_queue:
            return
        try:
            tracks = [self._track_from_dict(d) for d in raw_queue if isinstance(d, dict)]
        except Exception:  # noqa: BLE001
            return
        if not tracks:
            return
        start = int(state.get("index") or 0)
        self.queue.set_items(tracks, start=min(start, len(tracks) - 1))
        if state.get("shuffle"):
            self.action_shuffle()
        if state.get("repeat"):
            self.action_repeat()

    @staticmethod
    def _track_to_dict(track: Track) -> dict:
        return {
            "title": track.title,
            "artist": track.artist,
            "album": track.album,
            "duration": track.duration,
            "uri": track.uri,
            "provider": track.provider.value,
            "track_id": track.track_id,
            "cover_uri": track.cover_uri,
        }

    @staticmethod
    def _track_from_dict(d: dict) -> Track:
        return Track(
            title=str(d.get("title") or "?"),
            artist=str(d.get("artist") or ""),
            album=str(d.get("album") or ""),
            duration=float(d.get("duration") or 0.0),
            uri=str(d.get("uri") or ""),
            provider=Source(d.get("provider") or "local"),
            track_id=str(d.get("track_id") or ""),
            cover_uri=d.get("cover_uri"),
        )

    # ------------------------------------------------------------------ events
    @on(Button.Pressed, "#src-local")
    def _go_local(self) -> None:
        self._set_source(Source.LOCAL)

    @on(Button.Pressed, "#src-youtube")
    def _go_youtube(self) -> None:
        self._set_source(Source.YOUTUBE)

    @on(Button.Pressed, "#btn-queue")
    def _queue_btn(self) -> None:
        self.action_toggle_queue()

    @on(Button.Pressed, "#btn-play")
    def _btn_play(self) -> None:
        self.action_play_pause()

    @on(Button.Pressed, "#btn-next")
    def _btn_next(self) -> None:
        self.action_next()

    @on(Button.Pressed, "#btn-prev")
    def _btn_prev(self) -> None:
        self.action_prev()

    @on(Button.Pressed, "#btn-shuffle")
    def _btn_shuffle(self) -> None:
        self.action_shuffle()

    @on(Button.Pressed, "#btn-repeat")
    def _btn_repeat(self) -> None:
        self.action_repeat()

    @on(Button.Pressed, "#btn-pitch")
    def _btn_pitch(self) -> None:
        self.action_toggle_pitch()

    @on(NowPlaying.VolumeChanged)
    def _np_volume(self, message: NowPlaying.VolumeChanged) -> None:
        if not self.player:
            return
        try:
            self.player.set_volume(message.volume)
        except MpvError as e:
            self.notify(f"Volume error: {e}", severity="error")

    @on(NowPlaying.SpeedChanged)
    def _np_speed(self, message: NowPlaying.SpeedChanged) -> None:
        if not self.player:
            return
        try:
            self.player.set_speed(message.speed)
        except MpvError as e:
            self.notify(f"Speed error: {e}", severity="error")

    @on(NowPlaying.ReverbChanged)
    def _np_reverb(self, message: NowPlaying.ReverbChanged) -> None:
        if not self.player:
            return
        try:
            self.player.set_reverb(message.level)
        except MpvError as e:
            self.notify(f"Reverb error: {e}", severity="error")

    def _set_source(self, source: Source) -> None:
        if source not in ALL_SOURCES:
            return
        # Exit playlist mode when switching sources
        if self._playlist_mode:
            self._exit_playlist_mode()
        self._source = source
        for s in ALL_SOURCES:
            self.query_one(f"#src-{s.value}", Button).set_class(s is source, "active")
        if source is Source.LOCAL:
            asyncio.create_task(self._load_local())
        else:
            self.notify(f"Search in {source.value} — type below and press Enter", timeout=2)
            self.query_one(Input).focus()

    @on(Input.Submitted, "#searchbar")
    async def _on_search(self, event: Input.Submitted) -> None:
        query = event.value.strip()
        if not query:
            return
        prov = self._manager.by_source(self._source)
        if not prov.available:
            self.notify(f"{self._source.value} provider not available", severity="warning")
            return
        self.notify(f'Searching {self._source.value} for "{query}"…', timeout=8)
        try:
            tracks = await asyncio.to_thread(prov.search, query)
        except Exception as exc:  # noqa: BLE001
            self.notify(f"Search failed: {exc}", severity="error")
            return
        # Show heart button for YouTube results
        show_heart = self._source is Source.YOUTUBE
        on_dl = self._download_track if self._source is Source.YOUTUBE else None
        self.query_one("#list-view", TrackList).set_tracks(
            tracks, show_heart=show_heart,
            on_heart=self._add_track_to_playlist,
            on_download=on_dl,
        )
        self.notify(f"{len(tracks)} results", timeout=2)

    # ---- playlist actions ------------------------------------------------------
    def action_toggle_playlist_mode(self) -> None:
        if self._playlist_mode:
            self._exit_playlist_mode()
        else:
            self.query_one("#playlist-list", ListView).focus()
            self.notify("Use ↑/↓ to select playlist, Enter to open", timeout=2)

    def action_new_playlist(self) -> None:
        async def _create() -> None:
            name = await self.push_screen_wait(PlaylistInputScreen("New Playlist"))
            if name:
                self._config.create_playlist(name)
                self._refresh_playlist_sidebar()
                self.notify(f"Created playlist: {name}", timeout=2)

        self.run_worker(_create())

    def action_heart_track(self) -> None:
        """Add currently selected track to a playlist (H key)."""
        lv = self._active_list()
        if lv and lv.selected:
            self._add_track_to_playlist(lv.selected)

    def action_rename_playlist(self) -> None:
        if self._current_playlist_index is None:
            self.notify("Open a playlist first (Enter on playlist in sidebar)", timeout=2)
            return
        playlist = self._config.playlists[self._current_playlist_index]
        async def _rename() -> None:
            name = await self.push_screen_wait(PlaylistInputScreen("Rename Playlist", playlist["name"]))
            if name and name != playlist["name"]:
                self._config.rename_playlist(self._current_playlist_index, name)
                self._refresh_playlist_sidebar()
                self.notify(f"Renamed to: {name}", timeout=2)
        self.run_worker(_rename())

    def action_delete_playlist(self) -> None:
        if self._current_playlist_index is None:
            self.notify("Open a playlist first (Enter on playlist in sidebar)", timeout=2)
            return
        playlist = self._config.playlists[self._current_playlist_index]
        self._config.delete_playlist(self._current_playlist_index)
        self._exit_playlist_mode()
        self._refresh_playlist_sidebar()
        self.notify(f"Deleted playlist: {playlist['name']}", timeout=2)

    def _on_playlist_selected(self, index: int) -> None:
        """Called when user selects a playlist in sidebar (Enter)."""
        self._enter_playlist_mode(index)

    # ------------------------------------------------------------------ events
    @on(ListView.Selected, "#list-view-items")
    def _local_selected(self, event: ListView.Selected) -> None:
        item = event.item
        if isinstance(item, (TrackListItem, PlaylistTrackItem)):
            self._queue_from_active_list()
            asyncio.create_task(self.play_track(item.track))

    @on(ListView.Selected, "#queue-view-items")
    def _queue_selected(self, event: ListView.Selected) -> None:
        item = event.item
        if not isinstance(item, TrackListItem):
            return
        index = self.query_one("#queue-view", TrackList).index
        if index is not None:
            self.queue.jump(index)
            asyncio.create_task(self.play_track(item.track))

    def on_unmount(self) -> None:
        if self.player:
            self.player.stop()
        try:
            self._config.save_state(
                {
                    "queue": [self._track_to_dict(t) for t in self.queue.items],
                    "index": self.queue.position,
                    "shuffle": self.queue.shuffle,
                    "repeat": self.queue.repeat,
                }
            )
        except Exception:  # noqa: BLE001
            pass
