Write-Host "Installing Enigmatic Player..."
$ErrorActionPreference = "Stop"

if (-not (Get-Command python -ErrorAction SilentlyContinue)) {
    Write-Host "Python 3.10+ is required. Install it first." -ForegroundColor Red
    exit 1
}

$VenvDir = "$env:LOCALAPPDATA\enigmatic-player\venv"
Write-Host "Setting up virtual environment in ${VenvDir}..."
python -m venv "$VenvDir"
& "$VenvDir\Scripts\python.exe" -m pip install --upgrade pip
& "$VenvDir\Scripts\python.exe" -m pip install "git+https://github.com/lmdelm-dev/music-player.git"

# Create a launcher script
$Launcher = "$VenvDir\epm.bat"
@"
@echo off
"$VenvDir\Scripts\enigmatic.exe" %*
"@ | Out-File -FilePath $Launcher -Encoding ASCII

# Add to PATH for this session
$env:PATH = "$VenvDir;$env:PATH"

Write-Host ""
Write-Host "Done! Run 'epm' to launch Enigmatic Player." -ForegroundColor Green
Write-Host "Add $VenvDir to your PATH to use 'epm' from anywhere."
