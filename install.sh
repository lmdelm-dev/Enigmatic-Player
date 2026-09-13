#!/usr/bin/env bash
# Enigmatic Player — Linux/macOS install helper
set -euo pipefail

PYTHON="${PYTHON:-python3}"
EXTRAS="${EXTRAS:-youtube,art}"

if ! command -v "$PYTHON" >/dev/null 2>&1; then
    echo "Python 3.10+ is required. Install it first." >&2
    exit 1
fi

if ! command -v mpv >/dev/null 2>&1; then
    echo "mpv is required:"
    echo "  Debian/Ubuntu: sudo apt install mpv"
    echo "  Fedora:        sudo dnf install mpv"
    echo "  Arch:          sudo pacman -S mpv"
    echo "  macOS:         brew install mpv"
    echo "Install mpv, then re-run this script."
    exit 1
fi

DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
VENV_DIR="${VENV_DIR:-${VIRTUAL_ENV:-$DIR/.venv}}"
"$PYTHON" -c 'import sys; sys.exit(0 if sys.version_info >= (3, 10) else "Python 3.10+ is required.")'
if [[ ! -x "$VENV_DIR/bin/python" ]]; then
    "$PYTHON" -m venv "$VENV_DIR"
fi
PYTHON="$VENV_DIR/bin/python"
echo "Installing Enigmatic Player from $DIR …"
"$PYTHON" -m pip install -e "$DIR[$EXTRAS]"

echo
echo "Done! Activate with: source \"$VENV_DIR/bin/activate\""
echo "Then launch with:   enigmatic"
echo "Get help with:      enigmatic --help"