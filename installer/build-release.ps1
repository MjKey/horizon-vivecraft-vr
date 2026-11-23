# Makes HorizonVR-Installer.zip: the prebuilt game (from a Horizon checkout with the VR port,
# built with `cargo build --profile fast -p horizon-game`) and the installer window.
# The checkout is at HORIZON_BASE's commit (the Horizon launcher version the game plays on).
param([string]$Horizon = (Join-Path $PSScriptRoot '..\..\hz-nightly'))
$ErrorActionPreference = 'Stop'
# Each release needs a higher VERSION than the latest one on GitHub, or the installers already out
# there won't see it as an update.
$version = [version](Get-Content (Join-Path $PSScriptRoot 'VERSION') -Raw).Trim()
try {
    [Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12
    $latest = [version]((Invoke-RestMethod -UseBasicParsing -TimeoutSec 15 'https://api.github.com/repos/v1nx0/horizon-vivecraft-vr/releases/latest').tag_name.TrimStart('v'))
    if ($version -le $latest) { throw "VERSION is $version but GitHub already has ${latest}: raise installer\VERSION first." }
} catch [System.Net.WebException] {
    Write-Warning "Couldn't check GitHub's latest release: $_"
}
Write-Host "Building Horizon VR $version (publish it with the tag v$version)"
$out = Join-Path $PSScriptRoot 'out'
$stage = Join-Path $out 'HorizonVR-Installer'
$files = Join-Path $stage 'files'
Remove-Item -Recurse -Force $out -ErrorAction SilentlyContinue
New-Item -ItemType Directory -Force $files | Out-Null

foreach ($f in 'horizon-client.exe', 'horizon-prepare.exe', 'openxr_loader.dll') {
    Copy-Item (Join-Path $Horizon "target\fast\$f") $files
}
Copy-Item (Join-Path $Horizon 'research-lock.json') $files
Copy-Item (Join-Path $Horizon 'LICENSE') (Join-Path $files 'LICENSE.txt')
# The preparer keys its shipped exporter classes on these sources, so they ship too.
$parity = 'mods\horizon\src\test\java\dev\horizonlab\parity'
New-Item -ItemType Directory -Force (Join-Path $files (Split-Path $parity)) | Out-Null
Copy-Item -Recurse (Join-Path $Horizon $parity) (Join-Path $files $parity)
New-Item -ItemType Directory -Force (Join-Path $files 'crates\horizon-prepare\src') | Out-Null
Copy-Item (Join-Path $Horizon 'crates\horizon-prepare\src\exporters.rs') (Join-Path $files 'crates\horizon-prepare\src')
# The compiled exporters: the cache entry whose key matches (made by a prepare in the checkout).
$prepare = Join-Path $Horizon 'target\fast\horizon-prepare.exe'
# compile-exporters prints, last, the folder whose key matches this checkout.
$cache = (& $prepare compile-exporters --root $Horizon | Select-Object -Last 1).Trim()
if ($LASTEXITCODE -or -not (Test-Path (Join-Path $cache 'manifest.json'))) { throw 'compile-exporters failed' }
Copy-Item -Recurse $cache (Join-Path $files 'exporters')
Copy-Item (Join-Path $PSScriptRoot 'Play Horizon VR.bat') $files
# The release's version, which the installer's Update compares with GitHub's latest release tag.
Copy-Item (Join-Path $PSScriptRoot 'VERSION') (Join-Path $files 'version.txt')
# The Horizon launcher version this game is built on: the installer imports the game on it.
Copy-Item (Join-Path $PSScriptRoot 'HORIZON_BASE') (Join-Path $files 'horizon-base.txt')

# The installer: beside files\ to start, in files\ for the install folder (an older installer's
# Update copies only files\). The .ps1 and .bat stay for those older installers' Update.
# cargo writes its progress to stderr, which Stop would take for an error.
$ErrorActionPreference = 'Continue'
cargo build --release --manifest-path (Join-Path $PSScriptRoot 'app\Cargo.toml') 2>&1 | Out-Host
$ErrorActionPreference = 'Stop'
if ($LASTEXITCODE) { throw 'building the installer failed' }
$exe = Join-Path $PSScriptRoot 'app\target\release\HorizonVR-Installer.exe'
Copy-Item $exe $stage
Copy-Item $exe $files
Copy-Item (Join-Path $PSScriptRoot 'HorizonVR-Installer.ps1'), (Join-Path $PSScriptRoot 'Install Horizon VR.bat') $stage
Compress-Archive -Path $stage -DestinationPath (Join-Path $out 'HorizonVR-Installer.zip')
Write-Host "Made $(Join-Path $out 'HorizonVR-Installer.zip')"

