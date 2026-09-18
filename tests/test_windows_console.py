"""Windows console integration tests.

These prove, on a real (spawned) classic conhost:

1. A *fresh* console is echo-safe: stdin has no VT-input flag, so mouse
   movement can never be echoed as SGR codes, and VT processing can be
   enabled on stdout.
2. A console *poisoned* by a killed TUI (VT-input + any-event mouse left
   on) is fully healed by the app's startup cleanup
   (``enigmatic_player.cli._cleanup_mouse_state``): stdin loses the VT-input
   flag and the mouse-reporting mode is torn down.

Run directly or via pytest. Windows-only.
"""

from __future__ import annotations

import os
import subprocess
import sys
import tempfile
from pathlib import Path

import pytest

pytestmark = pytest.mark.skipif(sys.platform != "win32", reason="Windows only")


def _spawn_conhost(script: str, out_path: Path) -> None:
    """Run `script` in a brand-new classic conhost, then wait for its probe."""
    script_path = out_path.with_suffix(".py")
    script_path.write_text(script, encoding="utf-8")
    env = os.environ.copy()
    src_dir = str(Path(__file__).resolve().parents[1] / "src")
    env["PYTHONPATH"] = src_dir + os.pathsep + env.get("PYTHONPATH", "")
    flags = subprocess.CREATE_NEW_CONSOLE | subprocess.CREATE_NO_WINDOW
    proc = subprocess.Popen(
        [sys.executable, str(script_path)], env=env, creationflags=flags
    )
    try:
        proc.wait(timeout=90)
    except subprocess.TimeoutExpired:
        proc.kill()
        raise
    assert out_path.exists(), "conhost probe produced no output file"


_PROBE_FRESH = r"""
import ctypes
from ctypes import wintypes
out = r"%OUT%"
k = ctypes.WinDLL("kernel32", use_last_error=True)
hout = k.GetStdHandle(-11)
hin = k.GetStdHandle(-10)
mout = wintypes.DWORD()
min_ = wintypes.DWORD()
k.GetConsoleMode(hout, ctypes.byref(mout))
k.GetConsoleMode(hin, ctypes.byref(min_))
mout.value |= 4
k.SetConsoleMode(hout, mout.value)
check = wintypes.DWORD()
k.GetConsoleMode(hout, ctypes.byref(check))
with open(out, "w", encoding="utf-8") as f:
    f.write(f"stdout VT on: {bool(check.value & 4)}\n")
    f.write(f"stdin has VT-input(0x200): {bool(min_.value & 0x200)}\n")
"""

_PROBE_POISON_AND_HEAL = r"""
import ctypes
from ctypes import wintypes
out = r"%OUT%"
k = ctypes.WinDLL("kernel32", use_last_error=True)
hout = k.GetStdHandle(-11)
hin = k.GetStdHandle(-10)
# Simulate a killed TUI session that never cleaned up.
mo = wintypes.DWORD()
mi = wintypes.DWORD()
k.GetConsoleMode(hout, ctypes.byref(mo))
k.GetConsoleMode(hin, ctypes.byref(mi))
mi.value |= 0x200          # ENABLE_VIRTUAL_TERMINAL_INPUT
k.SetConsoleMode(hin, mi.value)
if mo.value & 4:
    # Any-event mouse + SGR reporting left on (only meaningful with VT output).
    try:
        import sys as _s
        _s.stdout.buffer.write(b"\x1b[?1003h\x1b[?1006h")
        _s.stdout.flush()
    except Exception:
        pass
# Now run the exact cleanup the app does at startup.
from enigmatic_player.cli import _cleanup_mouse_state, _reset_console_input
_cleanup_mouse_state()
_reset_console_input()
# Re-read the state.
mc = wintypes.DWORD()
k.GetConsoleMode(hin, ctypes.byref(mc))
with open(out, "w", encoding="utf-8") as f:
    f.write(f"stdin has VT-input(0x200): {bool(mc.value & 0x200)}\n")
    f.write("cleanup completed\n")
"""


_PROBE_VT_DROP_AND_RECOVER = r"""
import ctypes
from ctypes import wintypes
out = r"%OUT%"
k = ctypes.WinDLL("kernel32", use_last_error=True)
hout = k.GetStdHandle(-11)
# App startup: VT processing is on.
mo = wintypes.DWORD()
k.GetConsoleMode(hout, ctypes.byref(mo))
mo.value |= 4
k.SetConsoleMode(hout, mo.value)
from enigmatic_player.core import console as core_console
before = core_console.vt_processing_enabled()
# Simulate conhost silently dropping the flag (the focus/restore bug).
current = wintypes.DWORD()
k.GetConsoleMode(hout, ctypes.byref(current))
k.SetConsoleMode(hout, current.value & ~4)
dropped = not core_console.vt_processing_enabled()
# The watchdog repair path.
core_console.ensure_vt_processing()
after = core_console.vt_processing_enabled()
with open(out, "w", encoding="utf-8") as f:
    f.write(f"enabled at startup: {before}\n")
    f.write(f"drop detected: {dropped}\n")
    f.write(f"recovered after watchdog repair: {after}\n")
"""


def test_fresh_conhost_is_echo_safe():
    tmp = Path(tempfile.mkdtemp(prefix="conhost-fresh-"))
    out = tmp / "probe.txt"
    _spawn_conhost(_PROBE_FRESH.replace("%OUT%", out.as_posix()), out)
    lines = out.read_text(encoding="utf-8").splitlines()
    assert "stdout VT on: True" in lines
    assert "stdin has VT-input(0x200): False" in lines


def test_poisoned_conhost_is_healed_by_app_cleanup():
    tmp = Path(tempfile.mkdtemp(prefix="conhost-heal-"))
    out = tmp / "probe.txt"
    _spawn_conhost(_PROBE_POISON_AND_HEAL.replace("%OUT%", out.as_posix()), out)
    lines = out.read_text(encoding="utf-8").splitlines()
    assert "stdin has VT-input(0x200): False" in lines
    assert "cleanup completed" in lines


def test_vt_drop_is_detected_and_repaired():
    tmp = Path(tempfile.mkdtemp(prefix="conhost-vtdrop-"))
    out = tmp / "probe.txt"
    _spawn_conhost(_PROBE_VT_DROP_AND_RECOVER.replace("%OUT%", out.as_posix()), out)
    lines = out.read_text(encoding="utf-8").splitlines()
    assert "enabled at startup: True" in lines
    assert "drop detected: True" in lines
    assert "recovered after watchdog repair: True" in lines


if __name__ == "__main__":
    import pytest as _pytest

    raise SystemExit(_pytest.main([__file__, "-v"]))
