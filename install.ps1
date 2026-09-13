Write-Host "Installing Enigmatic Player..."
$ErrorActionPreference = "Stop"
if (-not (Get-Command python -ErrorAction SilentlyContinue)) {
    Write-Host "Python 3.10+ is required. Install it first." -ForegroundColor Red
    exit 1
}
python -m pip install --user --upgrade pip
python -m pip install --user "git+https://github.com/lmdelm-dev/music-player.git"
Write-Host "Done! Run 'epm' to launch Enigmatic Player." -ForegroundColor Green