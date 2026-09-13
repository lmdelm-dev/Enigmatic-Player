#!/bin/bash
set -euo pipefail

echo "Installing Enigmatic Player..."

# Check for Python 3.10+
if ! command -v python3 &>/dev/null; then
    echo "Error: Python 3.10+ is required. Install it first." >&2
    exit 1
fi

PYTHON_VERSION=$(python3 -c "import sys; print(f'{sys.version_info.major}.{sys.version_info.minor}')")
PYTHON_MAJOR=$(echo "$PYTHON_VERSION" | cut -d. -f1)
PYTHON_MINOR=$(echo "$PYTHON_VERSION" | cut -d. -f2)
if [ "$PYTHON_MAJOR" -lt 3 ] || { [ "$PYTHON_MAJOR" -eq 3 ] && [ "$PYTHON_MINOR" -lt 10 ]; }; then
    echo "Error: Python 3.10+ required (found $PYTHON_VERSION)." >&2
    exit 1
fi

# Check for mpv
if ! command -v mpv &>/dev/null; then
    echo "Warning: mpv is required for audio playback." >&2
    echo "  Install: sudo zypper install mpv  (openSUSE)" >&2
fi

VENV_DIR="${HOME}/.local/share/enigmatic-player/venv"
BIN_DIR="${HOME}/.local/bin"
WRAPPER="${BIN_DIR}/epm"

echo "Creating virtual environment at ${VENV_DIR}..."
mkdir -p "$(dirname "$VENV_DIR")"
python3 -m venv "$VENV_DIR"

echo "Installing dependencies..."
"${VENV_DIR}/bin/pip" install --upgrade pip
"${VENV_DIR}/bin/pip" install "git+https://github.com/lmdelm-dev/music-player.git"

# Create wrapper
mkdir -p "$BIN_DIR"
cat > "$WRAPPER" << EOF
#!/bin/bash
exec "${VENV_DIR}/bin/enigmatic" "\$@"
EOF
chmod +x "$WRAPPER"

echo ""
echo "Done! Run 'epm' to launch Enigmatic Player."
echo ""
if ! echo "$PATH" | grep -q "$BIN_DIR"; then
    echo "Add this to your shell profile (~/.bashrc or ~/.zshrc):"
    echo "  export PATH=\"\$HOME/.local/bin:\$PATH\""
    echo ""
    echo "Or run now:"
    echo "  export PATH=\"\$HOME/.local/bin:\$PATH\""
    echo "  epm"
else
    echo "Run: epm"
fi
