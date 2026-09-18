"""Console helpers for Windows: keep ANSI/VT processing enabled.

Legacy conhost on Windows 10 can silently drop ENABLE_VIRTUAL_TERMINAL_PROCESSING
from its output handle when the window loses focus, gets restored/maximized or
resized. When that happens mid-session, the next screen repaint is written as
raw escape codes — the "garbage" screen. These helpers make it cheap to re-assert
the flag repeatedly while the app runs.
"""

from __future__ import annotations

import sys

_ENABLE_VIRTUAL_TERMINAL_PROCESSING = 0x4


def vt_processing_enabled() -> bool:
    """True if stdout is a Windows console with VT processing currently on.

    Non-Windows platforms (and non-console handles like a pty/emulator) always
    report True: there the terminal handles ANSI itself, so nothing to check.
    """
    if sys.platform != "win32":
        return True
    try:
        import ctypes
        from ctypes import wintypes

        k32 = ctypes.WinDLL("kernel32", use_last_error=True)
        handle = k32.GetStdHandle(-11)  # STD_OUTPUT_HANDLE
        if not handle or handle == wintypes.HANDLE(-1).value:
            return True
        mode = wintypes.DWORD()
        if not k32.GetConsoleMode(handle, ctypes.byref(mode)):
            return True
        return bool(mode.value & _ENABLE_VIRTUAL_TERMINAL_PROCESSING)
    except Exception:  # noqa: BLE001 - never break the app over an env probe
        return True


def ensure_vt_processing() -> bool:
    """Force VT processing on for stdout; True if it's on afterwards.

    Returns False only for a genuine Windows console that refuses the flag
    (a legacy-console-mode window): callers can then refuse to start the TUI.
    """
    if sys.platform != "win32":
        return True
    try:
        import ctypes
        from ctypes import wintypes

        k32 = ctypes.WinDLL("kernel32", use_last_error=True)
        handle = k32.GetStdHandle(-11)  # STD_OUTPUT_HANDLE
        if not handle or handle == wintypes.HANDLE(-1).value:
            return True
        mode = wintypes.DWORD()
        if not k32.GetConsoleMode(handle, ctypes.byref(mode)):
            return True  # pty/emulator — the terminal handles ANSI itself
        mode.value |= _ENABLE_VIRTUAL_TERMINAL_PROCESSING
        k32.SetConsoleMode(handle, mode.value)
        check = wintypes.DWORD()
        if not k32.GetConsoleMode(handle, ctypes.byref(check)):
            return False
        return bool(check.value & _ENABLE_VIRTUAL_TERMINAL_PROCESSING)
    except Exception:  # noqa: BLE001 - best effort only
        return False

