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

REM Create launcher.
REM Written via Python because running this script through 'curl | cmd'
REM (stdin) makes cmd expand variables like an interactive prompt, so the
REM usual %% escaping produces a corrupt epm.bat. Python is required above,
REM and chr() lets us emit % and " without writing them literally here.
if not exist "%BIN_DIR%" mkdir "%BIN_DIR%"
python -c "import os,pathlib; q=chr(34); P=chr(37); NL=chr(10); BS=chr(92); R=os.path.join(os.environ['LOCALAPPDATA'],'enigmatic-player','repo'); V=R+os.sep+'.venv'; B=os.path.join(os.environ['LOCALAPPDATA'],'enigmatic-player','bin'); T='@echo off'+NL+'set REPO_DIR='+R+NL+'set VENV_DIR='+V+NL+NL+'REM Auto-update'+NL+'if exist '+q+P+'REPO_DIR'+P+BS+'.git'+q+' ('+NL+'    git -C '+q+P+'REPO_DIR'+P+q+' pull --ff-only >nul 2>&1'+NL+'    if '+q+P+'REPO_DIR'+P+BS+'pyproject.toml'+q+' GTR '+q+P+'VENV_DIR'+P+BS+'.installed'+q+' ('+NL+'        '+q+P+'VENV_DIR'+P+BS+'Scripts'+BS+'pip.exe'+q+' install -e '+q+P+'REPO_DIR'+P+'[youtube,art]'+q+' -q >nul 2>&1'+NL+'        type nul > '+q+P+'VENV_DIR'+P+BS+'.installed'+q+NL+'    )'+NL+')'+NL+NL+q+P+'VENV_DIR'+P+BS+'Scripts'+BS+'enigmatic.exe'+q+' '+P+'*'; pathlib.Path(os.path.join(B,'epm.bat')).write_text(T)"

echo.
echo Done! Run 'epm' to launch Enigmatic Player.
echo Add %BIN_DIR% to your System PATH to use 'epm' from anywhere.
echo   System Settings ^> Environment Variables ^> Path ^> Edit ^> New
echo   Paste: %BIN_DIR%
