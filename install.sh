#!/bin/bash
set -e

echo "Installing Enigmatic Player..."
if ! command -v python3 &>/dev/null; then
    echo "Python 3.10+ is required. Install it first." >&2
    exit 1
fi
python3 -m pip install --user --upgrade pip
python3 -m pip install --user "git+https://github.com/lmdelm-dev/music-player.git"
echo "Done! Run 'epm' to launch Enigmatic Player."