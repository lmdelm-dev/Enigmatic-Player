@echo off
echo Installing Enigmatic Player...
set VENV_DIR=%LOCALAPPDATA%\enigmatic-player\venv
echo Setting up virtual environment in %VENV_DIR%...
python -m venv "%VENV_DIR%"
"%VENV_DIR%\Scripts\python.exe" -m pip install --upgrade pip
"%VENV_DIR%\Scripts\python.exe" -m pip install "git+https://github.com/lmdelm-dev/music-player.git"
echo.
echo Done! Run '%VENV_DIR%\Scripts\enigmatic.exe' to launch Enigmatic Player.
echo Add %VENV_DIR%\Scripts to your PATH to use 'enigmatic' from anywhere.
