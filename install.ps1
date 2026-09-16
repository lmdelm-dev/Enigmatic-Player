Write-Host "Installing Enigmatic Player..."
$ErrorActionPreference = "Stop"

if (-not (Get-Command python -ErrorAction SilentlyContinue)) {
    Write-Host "Python 3.10+ is required. Install it first." -ForegroundColor Red
    exit 1
}

if (-not (Get-Command git -ErrorAction SilentlyContinue)) {
    Write-Host "Git is required. Install it first." -ForegroundColor Red
    exit 1
}

# Check for mpv (playback) and ffmpeg (YouTube MP3 downloads)
foreach ($bin in @("mpv", "ffmpeg")) {
    if (-not (Get-Command $bin -ErrorAction SilentlyContinue)) {
        $desc = if ($bin -eq "mpv") { "audio playback engine (required)" } else { "YouTube audio downloads / MP3 conversion (required for downloads)" }
        Write-Host "Warning: '$bin' not found — $desc" -ForegroundColor Yellow
        Write-Host "  Install with:  winget install -e --id mpv-player.mpv-CI.MSVC   or   winget install -e --id Gyan.FFmpeg" -ForegroundColor Yellow
    }
}

$InstallDir = "$env:LOCALAPPDATA\enigmatic-player"
$RepoDir = "$InstallDir\repo"
$VenvDir = "$RepoDir\.venv"
$BinDir = "$InstallDir\bin"
$RepoUrl = "https://github.com/lmdelm-dev/music-player.git"

# Clone or update repo
if (Test-Path "$RepoDir\.git") {
    Write-Host "Updating to latest version..."
    git -C "$RepoDir" pull --ff-only 2>$null
} else {
    Write-Host "Cloning repository..."
    git clone $RepoUrl $RepoDir
}

# Create or update venv
if (-not (Test-Path "$VenvDir\Scripts\python.exe")) {
    Write-Host "Setting up virtual environment..."
    python -m venv $VenvDir
}

Write-Host "Installing dependencies..."
& "$VenvDir\Scripts\pip.exe" install --upgrade pip -q
& "$VenvDir\Scripts\pip.exe" install -e "$RepoDir[youtube,art]" -q

# Create launcher
if (-not (Test-Path $BinDir)) { New-Item -ItemType Directory -Path $BinDir -Force | Out-Null }
$Launcher = "$BinDir\epm.bat"
@"
@echo off
set REPO_DIR=$RepoDir
set VENV_DIR=$VenvDir

REM Auto-update
if exist "%REPO_DIR%\.git" (
    git -C "%REPO_DIR%" pull --ff-only >nul 2>&1
    if "%REPO_DIR%\pyproject.toml" GTR "%VENV_DIR%\.installed" (
        "%VENV_DIR%\Scripts\pip.exe" install -e "%REPO_DIR%[youtube,art]" -q >nul 2>&1
        type nul > "%VENV_DIR%\.installed"
    )
)

"%VENV_DIR%\Scripts\enigmatic.exe" %*
"@ | Out-File -FilePath $Launcher -Encoding ASCII

Write-Host ""
Write-Host "Done! Run 'epm' to launch Enigmatic Player." -ForegroundColor Green
Write-Host "Add $BinDir to your System PATH to use 'epm' from anywhere."
Write-Host "  System Settings > Environment Variables > Path > Edit > New"
Write-Host "  Paste: $BinDir"
