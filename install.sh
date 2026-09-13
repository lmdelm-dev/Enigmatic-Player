#!/bin/bash
set -e

echo "Installing Enigmatic Player..."
if ! command -v python3 &>/dev/null; then
    echo "Python 3.10+ is required. Install it first." >&2
    exit 1
fi

VENV_DIR="${HOME}/.local/share/enigmatic-player/venv"
echo "Setting up virtual environment in ${VENV_DIR}..."
python3 -m venv "${VENV_DIR}" 2>/dev/null || python3 -m venv --without-pip "${VENV_DIR}"
"${VENV_DIR}/bin/python" -m ensurepip --upgrade 2>/dev/null || true
"${VENV_DIR}/bin/python" -m pip install --upgrade pip
"${VENV_DIR}/bin/python" -m pip install "git+https://github.com/lmdelm-dev/music-player.git"

# Create a wrapper script
mkdir -p "${HOME}/.local/bin"
cat > "${HOME}/.local/bin/epm" << 'EOF'
#!/bin/bash
exec "${HOME}/.local/share/enigmatic-player/venv/bin/enigmatic" "$@"
EOF
chmod +x "${HOME}/.local/bin/epm"

echo ""
echo "Done! Run 'epm' to launch Enigmatic Player."
echo "Make sure ${HOME}/.local/bin is in your PATH."
