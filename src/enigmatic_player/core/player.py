"""mpv audio engine wrapper.

We drive a single long-lived ``mpv`` process over its JSON IPC protocol:

* on POSIX this is a unix domain socket (``--input-ipc-server``)
* on Windows the same option creates a named pipe, accessed via ctypes
  (no pywin32 dependency)

One reader thread is started which parses newline-delimited JSON and pushes
both MPV events and property updates into an internal :class:`queue.Queue`.
State is mirrored into :attr:`MpvPlayer.state` so the UI can read current
position / paused / title trivially while reacting to events for the rest.
"""

from __future__ import annotations

import json
import logging
import os
import queue
import shutil
import socket
import subprocess
import sys
import tempfile
import threading
import time
from typing import Any, Callable, Dict, Optional

log = logging.getLogger(__name__)


class MpvError(RuntimeError):
    pass


class _IPCBase:
    """Minimal line-delimited JSON pipe to the mpv process."""

    def connect(self) -> None:  # pragma: no cover - platform dependent
        raise NotImplementedError

    def send_raw(self, payload: str) -> None:
        raise NotImplementedError

    def close(self) -> None:
        if self._sock:
            try:
                self._sock.close()
            except OSError:
                pass
            self._sock = None


PROPERTY_RENAME = {
    "time-pos": "time_pos",
    "pause": "paused",
    "duration": "duration",
    "volume": "volume",
    "media-title": "media_title",
    "media-artist": "media_artist",
}


class _SocketIPC(_IPCBase):
    def __init__(self, path: str) -> None:
        self._path = path
        self._sock: Optional[socket.socket] = None
        self._buffer = b""

    def connect(self) -> None:
        for _ in range(400):
            s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
            try:
                s.connect(self._path)
                s.settimeout(0.5)
                self._sock = s
                return
            except (OSError, ConnectionRefusedError):  # noqa: PERF203
                s.close()
                time.sleep(0.05)
        raise MpvError("timed out connecting to mpv IPC socket")

    def send_raw(self, payload: str) -> None:
        if self._sock is None:
            raise MpvError("IPC socket not connected")
        self._sock.sendall(payload.encode("utf-8"))

    def recv_until_newline(self, timeout: float = 0.5) -> str:
        if self._sock is None:
            raise MpvError("IPC socket not connected")
        self._sock.settimeout(timeout)
        while b"\n" not in self._buffer:
            chunk = self._sock.recv(65536)
            if not chunk:
                raise OSError("mpv IPC socket closed")
            self._buffer += chunk
        line, self._buffer = self._buffer.split(b"\n", 1)
        return line.decode("utf-8", "replace")


if sys.platform == "win32":
    import ctypes
    from ctypes import wintypes

    class _OVERLAPPED(ctypes.Structure):
        _fields_ = [
            ("Internal", ctypes.c_void_p),
            ("InternalHigh", ctypes.c_void_p),
            ("Offset", wintypes.DWORD),
            ("OffsetHigh", wintypes.DWORD),
            ("hEvent", wintypes.HANDLE),
        ]

    class _PipeIPC(_IPCBase):
        def __init__(self, name: str) -> None:
            self._name = name
            self._handle = None
            self._k32 = ctypes.WinDLL("kernel32", use_last_error=True)

        def connect(self) -> None:
            invalid = wintypes.HANDLE(-1).value
            for _ in range(400):
                handle = self._k32.CreateFileW(
                    self._name,
                    0xC0000000,  # GENERIC_READ | GENERIC_WRITE
                    0,
                    None,
                    3,  # OPEN_EXISTING
                    0x40000000,  # FILE_FLAG_OVERLAPPED for concurrent RW
                    None,
                )
                # CreateFileW returns INVALID_HANDLE_VALUE (-1) on failure;
                # ctypes may return it as -1 (signed) or 0xFFFFFFFF... (unsigned),
                # so check both representations.
                if handle != invalid and handle != -1:
                    self._handle = handle
                    return
                time.sleep(0.05)
            raise MpvError("timed out connecting to mpv IPC named pipe")

        def send_raw(self, payload: str) -> None:
            data = payload.encode("utf-8")
            written = wintypes.DWORD(0)
            ov = _OVERLAPPED()
            ov.hEvent = self._k32.CreateEventW(None, True, False, None)
            try:
                ok = self._k32.WriteFile(
                    self._handle, data, len(data), ctypes.byref(written), ctypes.byref(ov)
                )
                if not ok:
                    err = ctypes.get_last_error()
                    if err == 997:  # ERROR_IO_PENDING
                        res = self._k32.WaitForSingleObject(ov.hEvent, 5000)
                        if res != 0:  # WAIT_OBJECT_0
                            raise MpvError("mpv pipe write timed out")
                        ok2 = self._k32.GetOverlappedResult(
                            self._handle, ctypes.byref(ov), ctypes.byref(written), False
                        )
                        if not ok2:
                            raise MpvError("Failed writing to mpv pipe")
                    else:
                        raise MpvError("Failed writing to mpv pipe")
            finally:
                self._k32.CloseHandle(ov.hEvent)

        def recv_until_newline(self) -> str:
            buf = ctypes.create_string_buffer(65536)
            out = []
            while True:
                read = wintypes.DWORD(0)
                ov = _OVERLAPPED()
                ov.hEvent = self._k32.CreateEventW(None, True, False, None)
                try:
                    ok = self._k32.ReadFile(
                        self._handle, buf, len(buf), ctypes.byref(read), ctypes.byref(ov)
                    )
                    if not ok:
                        err = ctypes.get_last_error()
                        if err == 997:  # ERROR_IO_PENDING
                            res = self._k32.WaitForSingleObject(ov.hEvent, 30000)
                            if res != 0:
                                raise MpvError("timed out waiting for mpv IPC data")
                            ok2 = self._k32.GetOverlappedResult(
                                self._handle, ctypes.byref(ov), ctypes.byref(read), False
                            )
                            if not ok2:
                                raise OSError("mpv pipe closed")
                        else:
                            raise OSError("mpv pipe closed")
                    # success, read.value bytes available
                    if read.value == 0:
                        raise OSError("mpv pipe closed")
                    chunk = buf.raw[: read.value]
                    out.append(chunk.decode("utf-8", "replace"))
                    if b"\n" in chunk:
                        break
                finally:
                    self._k32.CloseHandle(ov.hEvent)
            text = "".join(out)
            return text.split("\n")[0]  # single complete line

        def close(self) -> None:
            if self._handle:
                self._k32.CloseHandle(self._handle)
                self._handle = None

    def _make_ipc(path: str) -> _IPCBase:
        return _PipeIPC(rf"\\.\pipe\{path}")

else:

    def _make_ipc(path: str) -> _IPCBase:
        return _SocketIPC(path)


def _set(holder: Dict[str, Any], item: Dict[str, Any]) -> None:
    holder["value"] = item.get("data")


def _maybe_update(state: Dict[str, Any], key: str, value: Any) -> None:
    if isinstance(value, (int, float, str, bool)):
        state[key] = value


def _apply_metadata(state: Dict[str, Any], value: Any) -> None:
    """Copy ARTIST/TITLE from mpv's ``metadata`` property into state.

    mpv exposes a map of key/value pairs; the JSON IPC serializes it as a
    list of ``[key, value]`` pairs, so normalise both shapes.
    """
    if isinstance(value, list):
        value = {str(k): v for k, v in value if isinstance(v, str)}
    if not isinstance(value, dict):
        return
    for k, v in value.items():
        low = str(k).lower()
        if low in ("artist", "albumartist") and isinstance(v, str) and v:
            state["media_artist"] = v
        elif low == "title" and isinstance(v, str) and v:
            state["media_title"] = v


class MpvPlayer:
    """A background mpv subprocess controlled over JSON IPC.

    Events and property changes surface through a :class:`queue.Queue`;
    a snapshot of useful properties is kept in :attr:`state`.
    """

    _OBSERVED = ("time-pos", "duration", "pause", "volume", "media-title", "media-artist")

    def __init__(
        self,
        mpv_bin: Optional[str] = None,
        on_event: Optional[Callable[[Dict[str, Any]], None]] = None,
    ) -> None:
        self._mpv_bin = shutil.which(mpv_bin or "mpv")
        if not self._mpv_bin:
            raise MpvError(
                "mpv executable not found. Install it first "
                "(e.g. `sudo apt install mpv`, `brew install mpv`, or "
                "`winget install mpv`)."
            )
        self._on_event = on_event or (lambda item: None)
        self.events: "queue.Queue[Dict[str, Any]]" = queue.Queue()
        self.state: Dict[str, Any] = {
            "paused": False,
            "duration": 0.0,
            "time_pos": 0.0,
            "volume": 100,
            "speed": 1.0,
            "reverb": 0,
            "pitch_mode": "nightcore",
            "media_title": "",
            "media_artist": "",
            "ended": False,
        }
        self._proc: Optional[subprocess.Popen] = None
        self._ipc: Optional[_IPCBase] = None
        self._reader: Optional[threading.Thread] = None
        self._write_lock = threading.Lock()
        self._id = 0
        self._requests: Dict[int, Callable[[Dict[str, Any]], None]] = {}
        self._sock_path: Optional[str] = None
        self._running = False
        self._initialized = threading.Event()

    # ------------------------------------------------------------------ lifecycle
    def start(self) -> None:
        if self._running:
            return
        if sys.platform == "win32":
            name = f"enigmatic-mpv-{os.getpid()}"
            ipc_arg = name
        else:
            fd, path = tempfile.mkstemp(prefix="enigmatic-mpv-", suffix=".sock")
            os.close(fd)
            os.unlink(path)
            self._sock_path = path
            ipc_arg = path

        self._proc = subprocess.Popen(
            [
                self._mpv_bin,
                "--idle=yes",
                "--force-window=no",
                "--no-video",
                "--no-terminal",
                "--really-quiet",
                f"--input-ipc-server={ipc_arg}",
                "--volume=100",
            ],
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
        )
        self._ipc = _make_ipc(ipc_arg)
        try:
            self._ipc.connect()
        except (MpvError, OSError) as exc:
            self.stop()
            raise MpvError(f"Unable to connect to mpv: {exc}") from exc
        self._running = True
        self._reader = threading.Thread(target=self._read_loop, daemon=True)
        self._reader.start()
        for i, prop in enumerate(self._OBSERVED):
            self._send(["observe_property", i + 1, prop])

    def stop(self) -> None:
        try:
            self._send(["quit"])
        except Exception:  # noqa: BLE001
            pass
        try:
            if self._proc:
                self._proc.wait(timeout=3)
        except subprocess.TimeoutExpired:
            self._proc.kill()
            self._proc.wait()
        self._running = False
        if self._ipc:
            self._ipc.close()
        if self._sock_path and os.path.exists(self._sock_path):
            try:
                os.unlink(self._sock_path)
            except OSError:
                pass

    def __enter__(self) -> "MpvPlayer":
        self.start()
        return self

    def __exit__(self, *exc: Any) -> None:
        self.stop()

    # ------------------------------------------------------------------- commands
    def _send(self, command: list, callback: Optional[Callable[[Dict[str, Any]], None]] = None) -> int:
        if not (self._running and self._ipc):
            raise MpvError("mpv player is not running")
        self._id += 1
        rid = self._id
        if callback:
            self._requests[rid] = callback
        payload = json.dumps({"command": command, "request_id": rid}) + "\n"
        with self._write_lock:
            self._ipc.send_raw(payload)
        return rid

    def get_property(self, name: str, callback: Optional[Callable[[Any], None]] = None, timeout: float = 5.0) -> Any:
        """Request a property value; returns it synchronously (blocking) or
        fires ``callback(value)`` asynchronously from the reader thread."""
        if callback is None:
            holder: Dict[str, Any] = {"done": threading.Event(), "value": None}
            self._send(
                ["get_property", name],
                callback=lambda item: (_set(holder, item), holder["done"].set()),
            )
            holder["done"].wait(timeout)
            return holder["value"]
        self._send(
            ["get_property", name],
            callback=lambda item: callback(item.get("data")),
        )
        return None

    def refresh_meta(self) -> None:
        """Ask mpv for duration/title out-of-band (observe misses some setters)."""
        if not self._running:
            return
        self.get_property("duration", callback=lambda v: _maybe_update(self.state, "duration", v))
        self.get_property("media-title", callback=lambda v: _maybe_update(self.state, "media_title", v))
        self.get_property("metadata", callback=lambda v: _apply_metadata(self.state, v))

    def load(self, uri: str, http_headers: Optional[Dict[str, str]] = None) -> None:
        """Load ``uri`` for playback.

        ``http_headers``: extra HTTP headers (e.g. a client-matched
        User-Agent from yt-dlp) required to fetch remote streams. Applied as
        a property *before* loadfile so they take effect on this load; they
        are harmless for local files and simply replaced on the next load.
        (Using set_property instead of per-file loadfile options keeps this
        compatible with older mpv releases.)
        """
        self.state.update({"media_title": "", "media_artist": "", "duration": 0.0,
                           "time_pos": 0.0, "ended": False})
        fields = [f"{key}: {value}" for key, value in (http_headers or {}).items()]
        self._send(["set_property", "http-header-fields", fields])
        self._send(["loadfile", uri, "replace"])
        # mpv only pushes these properties on *change*, so ask explicitly.
        self.get_property("duration", callback=lambda v: _maybe_update(self.state, "duration", v))
        self.get_property("media-title", callback=lambda v: _maybe_update(self.state, "media_title", v))
        self.get_property("metadata", callback=lambda v: _apply_metadata(self.state, v))
        # duration is only reliably reported a moment after the file opens
        threading.Timer(0.4, self.refresh_meta).start()

    def play(self) -> None:
        self._send(["set_property", "pause", False])

    def pause(self) -> None:
        self._send(["set_property", "pause", True])

    def toggle(self) -> bool:
        target = not bool(self.state.get("paused", False))
        self._send(["set_property", "pause", target])
        self.state["paused"] = target
        return target

    def seek(self, seconds: float, absolute: bool = False) -> None:
        mode = "absolute" if absolute else "relative"
        self._send(["seek", float(seconds), mode])

    def set_volume(self, volume: int) -> None:
        volume = max(0, min(150, int(volume)))
        self._send(["set_property", "volume", volume])
        self.state["volume"] = volume

    # ------------------------------------------------------------------- fx
    def _apply_audio_filters(self) -> None:
        """Rebuild mpv's ``af`` + pitch settings from current fx state.

        Two modes:

        * ``tempo``  — pitch is preserved (rubberband time-stretch), so
          slowing down / speeding up keeps voices natural (no chipmunk).
        * ``nightcore`` — pitch follows speed: mpv resamples instead of
          time-stretching, producing the classic sped-up higher-pitch sound.
        """
        mode = str(self.state.get("pitch_mode", "tempo"))
        speed = float(self.state.get("speed", 1.0))
        level = int(self.state.get("reverb", 0))

        if mode == "nightcore":
            # resample -> pitch shifts with speed; no rubberband, no scaletempo
            self._send(["set_property", "audio-pitch-correction", False])
            filters = []
        else:
            self._send(["set_property", "audio-pitch-correction", True])
            filters = []
            if abs(speed - 1.0) > 0.01:
                filters.append("rubberband")
        if level > 0:
            decay = 0.08 + 0.6 * (level / 100.0)
            filters.append(
                f"lavfi=[aecho=0.8:0.9:50|80|110:{decay:.2f}|{decay * 0.7:.2f}|{decay * 0.45:.2f}]"
            )
        chain = ",".join(filters)
        self._send(["set_property", "af", chain])

    def set_speed(self, speed: float) -> None:
        """Change playback speed in real time (0.5x .. 2.0x)."""
        speed = round(max(0.5, min(2.0, float(speed))), 2)
        self._send(["set_property", "speed", speed])
        self.state["speed"] = speed
        self._apply_audio_filters()

    def set_pitch_mode(self, mode: str) -> None:
        """Choose tempo (pitch-preserving) vs nightcore (pitch-shifted)."""
        if mode not in ("tempo", "nightcore"):
            return
        self.state["pitch_mode"] = mode
        self._apply_audio_filters()

    def adjust_speed(self, delta: float) -> float:
        speed = round(float(self.state.get("speed", 1.0)) + delta, 2)
        self.set_speed(speed)
        return speed

    def set_reverb(self, level: int) -> None:
        """Set reverb wetness 0..100 (0 = no filter)."""
        level = max(0, min(100, int(level)))
        self.state["reverb"] = level
        self._apply_audio_filters()

    def volume(self) -> int:
        return int(self.state.get("volume", 100))

    def next_media(self) -> None:
        self._send(["playlist-next", "weak"])

    @property
    def running(self) -> bool:
        return self._running

    # ------------------------------------------------------------------- reader
    def _read_loop(self) -> None:
        try:
            while self._running and self._proc and self._proc.poll() is None:
                if self._ipc is None:
                    break
                try:
                    line = self._ipc.recv_until_newline().strip()
                except socket.timeout:
                    continue  # idle/paused playback isn't a disconnect
                if not line:
                    continue
                try:
                    item: Dict[str, Any] = json.loads(line)
                except json.JSONDecodeError:
                    continue
                self._handle_item(item)
        except Exception as exc:  # noqa: BLE001
            log.debug("mpv IPC read loop ended: %s", exc)
        finally:
            if self._running:
                self._running = False
                self.events.put({"event": "disconnected"})
                self._on_event({"event": "disconnected"})

    def _handle_item(self, item: Dict[str, Any]) -> None:
        rid = item.get("request_id")
        if rid is not None and "event" not in item and rid in self._requests:
            cb = self._requests.pop(rid)
            try:
                cb(item)
            except Exception:  # noqa: BLE001
                log.debug("mpv property callback failed", exc_info=True)
            return
        etype = item.get("event")
        if etype == "property-change":
            name = item.get("name")
            data = item.get("data")
            key = PROPERTY_RENAME.get(name, name)
            if isinstance(data, (int, float, str, bool)):
                self.state[key] = data
            if key == "time_pos" and isinstance(data, (int, float)):
                self.state["ended"] = False
        elif etype == "end-file":
            reason = item.get("reason")
            if reason == "eof":
                self.state["ended"] = True
        self.events.put(item)
        self._on_event(item)

    def drain(self) -> list:
        out = []
        while True:
            try:
                out.append(self.events.get_nowait())
            except queue.Empty:
                break
        return out
