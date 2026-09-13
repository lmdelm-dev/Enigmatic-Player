@echo off
echo Installing Enigmatic Player...
set VENV_DIR=%LOCALAPPDATA%\enigmatic-player\venv

where python >nul 2>&1
if %errorlevel% neq 0 (
    echo Python 3.10+ is required. Install it first.
    exit /b 1
)

echo Setting up virtual environment in %VENV_DIR%...
python -m venv "%VENV_DIR%"
"%VENV_DIR%\Scripts\python.exe" -m pip install --upgrade pip
"%VENV_DIR%\Scripts\python.exe" -m pip install "git+https://github.com/lmdelm-dev/music-player.git"

REM Create epm launcher
echo @echo off > "%VENV_DIR%\Scripts\epm.bat"
echo "%VENV_DIR%\Scripts\enigmatic.exe" %%* >> "%VENV_DIR%\Scripts\epm.bat"

echo.
echo Done! Run 'epm' to launch Enigmatic Player.
echo Add %VENV_DIR%\Scripts to your System PATH to use 'epm' from anywhere.
echo   System Settings ^> Environment Variables ^> Path ^> Edit ^> New
echo   Paste: %VENV_DIR%\Scripts
