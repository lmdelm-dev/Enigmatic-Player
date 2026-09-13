# Enigmatic Player — Windows install helper (PowerShell)
$ErrorActionPreference = "Stop"

$Python = (Get-Command python -ErrorAction SilentlyContinue).Source
if (-not $Python) {
    Write-Error "Python 3.10+ is required. Install it from https://python.org/"
}

Write-Host "Checking for mpv..."
if (-not (Get-Command mpv -ErrorAction SilentlyContinue)) {
    Write-Host "mpv not found. Install it with:  winget install mpv"
    Write-Host "Then re-run this script."
    exit 1
}

$Dir = Split-Path -Parent $MyInvocation.MyCommand.Path
$Extras = "youtube,art"
$VenvDir = if ($env:VIRTUAL_ENV) { $env:VIRTUAL_ENV } else { Join-Path $Dir ".venv" }
& $Python -c 'import sys; sys.exit(0 if sys.version_info >= (3, 10) else "Python 3.10+ is required.")'
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
$VenvPython = Join-Path $VenvDir "Scripts\python.exe"
if (-not (Test-Path $VenvPython)) {
    & $Python -m venv $VenvDir
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
}

Write-Host "Installing Enigmatic Player from $Dir ..."
& $VenvPython -m pip install -e "${Dir}[$Extras]"
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

Write-Host ""
Write-Host "Done! Activate with: & `"$VenvDir\Scripts\Activate.ps1`""
Write-Host "Then launch with:   enigmatic"
Write-Host "Get help with:      enigmatic --help"
Write-Host ""
Write-Host "Note: if the TUI misrenders, enable Windows Terminal and set"
Write-Host "`TERM=xterm-256color` and a True Color / RGB color profile."