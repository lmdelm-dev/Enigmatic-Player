@echo off
echo Installing Enigmatic Player...
python -m pip install --user --upgrade pip
python -m pip install --user "git+https://github.com/lmdelm-dev/music-player.git"
echo Done! Run 'epm' to launch Enigmatic Player.