# Builds the Windows installer with Inno Setup.
#
#   powershell -File installer\build.ps1
#
# The release binary must exist first:
#   cargo build --release   (inside enigmatic-player-rust)

$ErrorActionPreference = 'Stop'
$here = Split-Path -Parent $MyInvocation.MyCommand.Path
$root = Split-Path -Parent $here

$exe = Join-Path $root 'enigmatic-player-rust\target\release\enigmatic-player.exe'
if (-not (Test-Path $exe)) {
    throw "missing $exe - run 'cargo build --release' in enigmatic-player-rust first"
}

$iscc = @(
    "$env:LOCALAPPDATA\Programs\Inno Setup 6\ISCC.exe",
    'C:\Program Files (x86)\Inno Setup 6\ISCC.exe',
    'C:\Program Files\Inno Setup 6\ISCC.exe'
) | Where-Object { Test-Path $_ } | Select-Object -First 1

if (-not $iscc) {
    throw 'ISCC.exe not found - install Inno Setup 6 (https://jrsoftware.org/isdl.php)'
}

Write-Host "binary : $exe ($([math]::Round((Get-Item $exe).Length / 1MB, 1)) MB)"
Write-Host "iscc   : $iscc"

& $iscc (Join-Path $here 'enigmatic-player.iss')
if ($LASTEXITCODE -ne 0) { throw "ISCC failed with exit code $LASTEXITCODE" }

Get-ChildItem (Join-Path $here 'dist') -Filter '*.exe' |
    Select-Object Name, Length |
    Format-Table -AutoSize
