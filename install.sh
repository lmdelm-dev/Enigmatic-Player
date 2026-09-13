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

INSTALL_DIR="${HOME}/.local/share/enigmatic-player"
REPO_DIR="${INSTALL_DIR}/repo"
VENV_DIR="${REPO_DIR}/.venv"
BIN_DIR="${HOME}/.local/bin"
WRAPPER="${BIN_DIR}/epm"
REPO_URL="https://github.com/lmdelm-dev/music-player.git"

# Clone or update repo
if [ -d "$REPO_DIR/.git" ]; then
    echo "Updating to latest version..."
    git -C "$REPO_DIR" pull --ff-only 2>/dev/null || true
else
    echo "Cloning repository..."
    mkdir -p "$(dirname "$REPO_DIR")"
    git clone "$REPO_URL" "$REPO_DIR"
fi

# Create or update venv
if [ ! -x "$VENV_DIR/bin/python" ]; then
    echo "Setting up virtual environment..."
    python3 -m venv "$VENV_DIR"
fi

echo "Installing dependencies..."
"$VENV_DIR/bin/pip" install --upgrade pip -q
"$VENV_DIR/bin/pip" install -e "$VENV_DIR/..[youtube,art]" -q

# Create wrapper
mkdir -p "$BIN_DIR"
cat > "$WRAPPER" << 'WRAPPER_EOF'
#!/bin/bash
REPO_DIR="${HOME}/.local/share/enigmatic-player/repo"
VENV_DIR="${REPO_DIR}/.venv"

# Auto-update: pull latest changes
if [ -d "$REPO_DIR/.git" ]; then
    git -C "$REPO_DIR" pull --ff-only 2>/dev/null || true
    # Reinstall if dependencies changed
    if [ "$REPO_DIR/pyproject.toml" -nt "$VENV_DIR/.installed" ]; then
        "$VENV_DIR/bin/pip" install -e "$REPO_DIR[youtube,art]" -q 2>/dev/null
        touch "$VENV_DIR/.installed"
    fi
fi

exec "$VENV_DIR/bin/enigmatic" "$@"
WRAPPER_EOF
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
