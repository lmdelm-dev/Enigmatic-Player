"""A small terminal slider (knob) for volume / speed / reverb.

Textual's built-in ``Slider`` is not available everywhere, so this is a
self-contained focusable widget that renders ``LB ▛██████░░░▟ 42`` and
handles mouse clicks + arrow / JKL keybinds. Friendly with headless tests.
"""

from __future__ import annotations

from textual.app import ComposeResult
from textual.events import Click, Key
from textual.message import Message
from textual.widget import Widget
from textual.widgets import Static

COLOR_TRACK = "#504945"
COLOR_FILL = "#8ec07c"
COLOR_TICK = "#b8bb26"


class Slider(Widget):
    """A single-axis adjustable slider shown as a horizontal bar."""

    can_focus = True

    class Changed(Message):
        def __init__(self, slider: "Slider") -> None:
            self.slider = slider
            super().__init__()

    def __init__(
        self,
        label: str = "",
        value: float = 0.0,
        maximum: float = 100.0,
        step: float = 1.0,
        format: str = "{:.0f}",
        **kwargs,
    ) -> None:
        super().__init__(**kwargs)
        self.label = label
        self._value = float(value)
        self.maximum = float(maximum)
        self.step = float(step)
        self._display = (lambda v: format.format(v)) if isinstance(format, str) else format
        self._bar = Static("", classes="slider-bar")

    # ------------------------------------------------------------------ state
    @property
    def value(self) -> float:
        return self._value

    @value.setter
    def value(self, val: float) -> None:
        val = min(self.maximum, max(0.0, float(val)))
        if abs(val - self._value) < 1e-9:
            return
        self._value = val
        self._refresh_bar()
        self.post_message(self.Changed(self))

    def set_value_silent(self, val: float) -> None:
        """Update the displayed position without emitting a change message."""
        val = min(self.maximum, max(0.0, float(val)))
        self._value = val
        self._refresh_bar()

    # ------------------------------------------------------------------ layout
    def compose(self) -> ComposeResult:
        yield self._bar

    def on_mount(self) -> None:
        self._refresh_bar()

    def on_resize(self) -> None:
        self._refresh_bar()

    def _refresh_bar(self) -> None:
        text = self._display(self._value)
        width = max(0, self.size.width - len(self.label) - len(text) - 3)
        frac = self._value / self.maximum if self.maximum else 0.0
        filled = int(round(frac * width))
        if filled > 0 and filled < width:
            bar = (
                f"[{COLOR_FILL}]{'█' * (filled - 1)}[/]"
                f"[{COLOR_TICK}]█[/]"
                f"[{COLOR_TRACK}]{'░' * (width - filled)}[/]"
            )
        else:
            bar = (
                f"[{COLOR_FILL}]{'█' * filled}[/]"
                f"[{COLOR_TRACK}]{'░' * (width - filled)}[/]"
            )
        self._bar.update(f"{self.label} {bar} {text}")

    # ------------------------------------------------------------------ input
    def _nudge(self, delta: float) -> None:
        self.value = self._value + delta * self.step

    def on_key(self, event: Key) -> None:
        if event.key == "left":
            self._nudge(-1.0)
        elif event.key == "right":
            self._nudge(1.0)
        elif event.key == "j":
            self._nudge(-1.0)
        elif event.key == "l":
            self._nudge(1.0)
        elif event.key == "k":
            self._nudge(-5.0)
        elif event.key == "h":
            self.value = 0.0
        elif event.key == "g":
            self.value = self.maximum
        else:
            return
        event.stop()

    def on_click(self, event: Click) -> None:
        rel_x = event.x - self._click_offset
        width = max(4, self.size.width - len(self.label) - 3)
        if width <= 0:
            return
        frac = min(1.0, max(0.0, rel_x / width))
        self.value = frac * self.maximum
        event.stop()

    @property
    def _click_offset(self) -> int:
        return len(self.label) + 1
