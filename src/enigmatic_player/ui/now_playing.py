"""Now Playing panel: art, meta, progress bar and transport controls."""

from __future__ import annotations

import math
from typing import Optional

from textual.app import ComposeResult
from textual.containers import Horizontal, Vertical
from textual.message import Message
from textual.reactive import reactive
from textual.widgets import Button, Label, Static

from ..core.track import Track, fmt_time
from .art import PLACEHOLDER, render_gameboy
from .slider import Slider

MUSIC_NOTE = "♫"
EQ_CHARS = "▁▂▃▄▅▆▇█"

# gruvbox palette
INK_BRIGHT = "#fbf1c7"
INK_MID = "#d5c4a1"
INK_DIM = "#928374"
GLOW = "#8ec07c"


class EqBars(Static):
    """An animated pseudo-equalizer — deterministic, no FFT needed."""

    active: reactive[bool] = reactive(False)
    tick: reactive[int] = reactive(0)

    def render(self) -> str:
        bars = []
        for i in range(16):
            if self.active:
                t = self.tick * 0.52
                v = (
                    math.sin(t + i * 1.31) * 0.5
                    + math.sin(t * 1.7 + i * 2.1) * 0.3
                    + math.sin(t * 2.9 + i * 0.6) * 0.2
                )
                level = (v + 1.0) / 2.0
                level = 0.16 + 0.84 * (level ** 1.55)
            else:
                level = 0.1 + 0.05 * math.sin(i * 2.4 + self.tick * 0.1)
            idx = min(7, int(level * 8))
            bars.append(EQ_CHARS[idx])
        color = GLOW if self.active else INK_DIM
        return f"[{color}]" + "".join(bars) + "[/]"


class ArtDisplay(Static):
    """A framed block showing pixel-art of the album cover."""

    can_focus = False

    def show_cover(self, art_bytes: Optional[bytes]) -> None:
        markup = render_gameboy(art_bytes) if art_bytes else None
        self.update(markup or PLACEHOLDER)


class Progress(Static):
    """Time + animated progress bar — wider, centered, with %."""

    position: reactive[float] = reactive(0.0)
    duration: reactive[float] = reactive(0.0)
    active: reactive[bool] = reactive(False)

    def render(self) -> str:
        if self.duration <= 0:
            shown = 0.0
            rhs = fmt_time(0)
            pct = "  0%"
        else:
            shown = min(self.position / self.duration, 1.0)
            rhs = fmt_time(self.duration)
            pct = f"{int(shown*100):3d}%"
        # 30 cells wide for 80-col layout
        width = max(10, int(shown * 30))
        fill = "█" * width
        rest = "░" * max(0, 30 - width)
        head = "▶" if self.active else "·"
        bar = f"[{GLOW}]{fill}[/][{INK_DIM}]{rest}[/] {head}"
        return f"[{INK_MID}]{fmt_time(self.position)}[/] {bar} [{INK_MID}]{rhs}[/] [{INK_DIM}]{pct}[/]"


class NowPlaying(Horizontal):
    """Bottom panel combining album art + metadata + controls."""

    title: reactive[str] = reactive("")
    artist: reactive[str] = reactive("")
    album: reactive[str] = reactive("")
    played: reactive[float] = reactive(0.0)
    length: reactive[float] = reactive(0.0)
    paused: reactive[bool] = reactive(True)
    _pulse_on: bool = True

    def compose(self) -> ComposeResult:
        with Vertical(id="art-wrap", classes="artbox"):
            yield ArtDisplay(PLACEHOLDER, id="art")
        with Vertical(id="meta", classes="np-meta"):
            yield Label("Nothing playing yet", id="np-title", classes="np-title")
            yield Label("", id="np-artist", classes="np-artist")
            yield Label("", id="np-album", classes="np-album")
            yield Progress(id="np-progress", classes="np-progress")
            yield EqBars("", id="np-eq", classes="np-eq")
            yield Label("○ IDLE", id="np-status", classes="np-status")
            yield Label("", id="np-fxstate", classes="np-fxstate")
        with Vertical(id="knobs-panel", classes="np-transport"):
            with Horizontal(classes="pt-row"):
                yield Button("|<", id="btn-prev", variant="default")
                yield Button(">", id="btn-play", variant="success", classes="pt-play")
                yield Button(">|", id="btn-next", variant="default")
            with Horizontal(classes="pt-row"):
                yield Button("S", id="btn-shuffle", variant="default")
                yield Button("R", id="btn-repeat", variant="default")
            yield Slider("VOL", value=100, maximum=100, id="sl-vol", classes="np-knob")
            yield Slider(
                "SPD",
                value=100,
                maximum=200,
                step=2,
                format=lambda v: f"{v / 100.0:.2f}x",
                id="sl-speed",
                classes="np-knob",
            )
            yield Slider("REV", value=0, maximum=100, id="sl-reverb", classes="np-knob")
            yield Button("PITCH: MIX", id="btn-pitch", variant="default", classes="pt-pitch")

    def on_mount(self) -> None:
        self.set_interval(0.28, self._pulse_tick)
        self.query_one("#sl-vol", Slider).set_value_silent(100)
        self.query_one("#sl-speed", Slider).set_value_silent(100)
        self.query_one("#sl-reverb", Slider).set_value_silent(0)

    def watch_title(self, value: str) -> None:
        title = self.query_one("#np-title", Label)
        if value:
            title.update(f"[bold {INK_BRIGHT}]{MUSIC_NOTE} {value}[/]")
            title.set_classes("np-title playing")
        else:
            title.update("Nothing playing yet")
            title.set_classes("np-title idle")
        self._pulse_title()

    def watch_artist(self, value: str) -> None:
        self.query_one("#np-artist", Label).update(value)

    def watch_album(self, value: str) -> None:
        self.query_one("#np-album", Label).update(value)

    def watch_played(self, value: float) -> None:
        self.query_one(Progress).position = value

    def watch_length(self, value: float) -> None:
        self.query_one(Progress).duration = value

    def watch_paused(self, value: bool) -> None:
        self.query_one("#btn-play", Button).label = ">" if value else "||"
        self.query_one(Progress).active = not self.paused and bool(self.title)
        self._render_status()

    # ---- property sync -------------------------------------------------------
    def sync_volume(self, volume: int) -> None:
        try:
            sl = self.query_one("#sl-vol", Slider)
            if not sl.has_focus:
                sl.set_value_silent(int(volume))
        except Exception:  # noqa: BLE001 - not mounted yet
            pass

    def sync_speed(self, speed: float) -> None:
        try:
            sl = self.query_one("#sl-speed", Slider)
            if not sl.has_focus:
                sl.set_value_silent(speed * 100.0)
        except Exception:  # noqa: BLE001
            pass

    def sync_fx(self, speed: float, reverb: int, pitch_mode: str = "tempo") -> None:
        try:
            label = self.query_one("#np-fxstate", Label)
            parts = []
            if abs(speed - 1.0) > 0.01:
                parts.append(f"SPD {speed:.2f}")
            if reverb > 0:
                parts.append(f"REV {reverb}%")
            if pitch_mode == "nightcore":
                parts.append("MIX")
            label.update(" • ".join(parts) if parts else "   ")
        except Exception:  # noqa: BLE001
            pass

    def sync_pitch_mode(self, mode: str) -> None:
        try:
            btn = self.query_one("#btn-pitch", Button)
            btn.label = "PITCH: MIX" if mode == "nightcore" else "PITCH: LOCK"
            btn.set_class(mode == "nightcore", "toggled")
        except Exception:  # noqa: BLE001
            pass

    # ---- slider messages -----------------------------------------------------
    def on_slider_changed(self, message: Slider.Changed) -> None:
        sl = message.slider
        if sl.id == "sl-vol":
            self.post_message(self.VolumeChanged(int(sl.value)))
        elif sl.id == "sl-speed":
            self.post_message(self.SpeedChanged(sl.value / 100.0))
        elif sl.id == "sl-reverb":
            self.post_message(self.ReverbChanged(int(sl.value)))
        self.sync_fx(
            speed=self.query_one("#sl-speed", Slider).value / 100.0,
            reverb=int(self.query_one("#sl-reverb", Slider).value),
        )

    class VolumeChanged(Message):
        def __init__(self, volume: int) -> None:
            self.volume = volume
            super().__init__()

    class SpeedChanged(Message):
        def __init__(self, speed: float) -> None:
            self.speed = speed
            super().__init__()

    class ReverbChanged(Message):
        def __init__(self, level: int) -> None:
            self.level = level
            super().__init__()

    # ---- animation -----------------------------------------------------------
    def tick_eq(self) -> None:
        try:
            eq = self.query_one(EqBars)
            eq.tick += 1
            eq.active = not self.paused and bool(self.title)
        except Exception:  # noqa: BLE001
            pass

    def tick_progress_head(self) -> None:
        try:
            prog = self.query_one(Progress)
            prog.active = not self.paused and bool(self.title)
        except Exception:  # noqa: BLE001
            pass

    def _playing(self) -> bool:
        return bool(self.title) and not self.paused

    def _pulse_tick(self) -> None:
        self._pulse_on = not self._pulse_on
        self._render_status()
        if self._playing():
            self._pulse_title()

    def _pulse_title(self) -> None:
        try:
            title = self.query_one("#np-title", Label)
            if self._playing():
                title.animate(
                    "text_opacity", 0.55 if self._pulse_on else 1.0,
                    duration=0.6, easing="in_out_cubic",
                )
            elif title.text_opacity != 1.0:
                title.text_opacity = 1.0
        except Exception:  # noqa: BLE001 - animation API is best-effort
            pass

    def _render_status(self) -> None:
        try:
            label = self.query_one("#np-status", Label)
            if not self.title:
                text, cls = "○ IDLE", ""
            elif self.paused:
                text, cls = "▮ PAUSED", ""
            else:
                dot = "●" if self._pulse_on else "○"
                text, cls = f"{dot} PLAYING", "playing"
            label.update(text)
            label.set_classes(f"np-status {cls}" if cls else "np-status")
        except Exception:  # noqa: BLE001
            pass

    def set_track(self, track: Optional[Track], paused: bool = True) -> None:
        if track:
            self.title = track.title
            self.artist = track.artist or ""
            self.album = track.album or ""
            self.played = 0.0
            self.length = track.duration or 0.0
            art = track.albumart
            self.query_one(ArtDisplay).show_cover(art)
            artbox = self.query_one("#art-wrap")
            artbox.set_classes("artbox with-art" if art else "artbox")
        else:
            self.title = ""
            self.artist = ""
            self.album = ""
            self.played = 0.0
            self.length = 0.0
            self.query_one(ArtDisplay).show_cover(None)
            self.query_one("#art-wrap").set_classes("artbox")
        self.paused = paused
