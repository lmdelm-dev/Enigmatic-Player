@echo off
echo Installing Enigmatic Player...

where python >nul 2>&1
if %errorlevel% neq 0 (
    echo Python 3.10+ is required. Install it first.
    exit /b 1
)

where git >nul 2>&1
if %errorlevel% neq 0 (
    echo Git is required. Install it first.
    exit /b 1
)

set INSTALL_DIR=%LOCALAPPDATA%\enigmatic-player
set REPO_DIR=%INSTALL_DIR%\repo
set VENV_DIR=%REPO_DIR%\.venv
set BIN_DIR=%INSTALL_DIR%\bin
set REPO_URL=https://github.com/lmdelm-dev/music-player.git

REM Clone or update repo
if exist "%REPO_DIR%\.git" (
    echo Updating to latest version...
    git -C "%REPO_DIR%" pull --ff-only >nul 2>&1
) else (
    echo Cloning repository...
    git clone %REPO_URL% "%REPO_DIR%"
)

REM Create or update venv
if not exist "%VENV_DIR%\Scripts\python.exe" (
    echo Setting up virtual environment...
    python -m venv "%VENV_DIR%"
)

echo Installing dependencies...
"%VENV_DIR%\Scripts\pip.exe" install --upgrade pip -q
"%VENV_DIR%\Scripts\pip.exe" install -e "%REPO_DIR%[youtube,art]" -q

REM Create launcher
if not exist "%BIN_DIR%" mkdir "%BIN_DIR%"
(
    echo @echo off
    echo set REPO_DIR=%REPO_DIR%
    echo set VENV_DIR=%VENV_DIR%
    echo.
    echo REM Auto-update
    echo if exist "%%REPO_DIR%%\.git" ^(
    echo     git -C "%%REPO_DIR%%" pull --ff-only ^>nul 2^>^&1
    echo     if "%%REPO_DIR%%\pyproject.toml" GTR "%%VENV_DIR%%\.installed" ^(
    echo         "%%VENV_DIR%%\Scripts\pip.exe" install -e "%%REPO_DIR%%[youtube,art]" -q ^>nul 2^>^&1
    echo         type nul ^> "%%VENV_DIR%%\.installed"
    echo     ^)
    echo ^)
    echo.
    echo "%%VENV_DIR%%\Scripts\enigmatic.exe" %%*
) > "%BIN_DIR%\epm.bat"

echo.
echo Done! Run 'epm' to launch Enigmatic Player.
echo Add %BIN_DIR% to your System PATH to use 'epm' from anywhere.
echo   System Settings ^> Environment Variables ^> Path ^> Edit ^> New
echo   Paste: %BIN_DIR%
