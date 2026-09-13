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

# Create epm launcher in the venv Scripts dir
$Launcher = "$VenvDir\Scripts\epm.bat"
$Content = "@echo off`r`n`"$VenvDir\Scripts\enigmatic.exe`" %*"
[System.IO.File]::WriteAllText($Launcher, $Content)

Write-Host ""
Write-Host "Done! Run 'epm' to launch Enigmatic Player." -ForegroundColor Green
Write-Host "Add $VenvDir\Scripts to your System PATH to use 'epm' from anywhere."
Write-Host "  System Settings > Environment Variables > Path > Edit > New"
Write-Host "  Paste: $VenvDir\Scripts"
