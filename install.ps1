# Installs Horizon with the Vivecraft VR port: downloads Horizon, adds the VR code, builds it and
# prepares the game files from your own Minecraft 26.3. Needs Git, Rust (rustup) and JDK 25.
$ErrorActionPreference = 'Stop'
$here = $PSScriptRoot
$horizon = Join-Path $here 'horizon'
# The Horizon commit the patch was made against.
$commit = 'c07bfa7'

function Need($cmd, $what, $url) {
    if (-not (Get-Command $cmd -ErrorAction SilentlyContinue)) {
        Write-Host "$what is missing. Install it from $url, then run this installer again." -ForegroundColor Red
        exit 1
    }
}
Need git 'Git' 'https://git-scm.com/download/win'
Need cargo 'Rust' 'https://rustup.rs'
Need javac 'JDK 25' 'https://adoptium.net'
$javaVersion = (& javac -version 2>&1 | Out-String) -replace '[^0-9.]', ''
if ([int]($javaVersion.Split('.')[0]) -lt 25) {
    Write-Host "JDK 25 is needed; javac is version $javaVersion. Install it from https://adoptium.net." -ForegroundColor Red
    exit 1
}

if (-not (Test-Path $horizon)) {
    Write-Host 'Downloading Horizon...'
    git clone https://github.com/horizon-rust/horizon.git $horizon
    if ($LASTEXITCODE) { exit 1 }
}
git -C $horizon checkout -q $commit
if ($LASTEXITCODE) { Write-Host "Can't find Horizon commit $commit." -ForegroundColor Red; exit 1 }

Write-Host 'Adding the VR code...'
Copy-Item -Recurse -Force (Join-Path $here 'crates\horizon-vr') (Join-Path $horizon 'crates\')
Copy-Item -Recurse -Force (Join-Path $here 'horizon-client-vr\vr') (Join-Path $horizon 'crates\horizon-client\src\game\')
# Skip the patch when an earlier run already applied it.
git -C $horizon apply --reverse --check (Join-Path $here 'horizon-hooks.patch') 2>$null
if ($LASTEXITCODE) {
    git -C $horizon apply (Join-Path $here 'horizon-hooks.patch')
    if ($LASTEXITCODE) { Write-Host 'The hooks patch did not apply.' -ForegroundColor Red; exit 1 }
}

Write-Host 'Building (this takes a while the first time)...'
Push-Location $horizon
try {
    cargo build --profile fast -p horizon-game
    if ($LASTEXITCODE) { Write-Host 'The build failed.' -ForegroundColor Red; exit 1 }

    if (-not (Test-Path '.local\horizon-client\26.3')) {
        Write-Host 'Preparing the game files from your Minecraft 26.3 install.'
        Write-Host 'You need the Minecraft 26.3 server jar (from minecraft.net).'
        $jar = (Read-Host 'Path to the 26.3 server jar').Trim('"')
        cargo run --profile fast -p horizon-prepare -- --prepare --server-jar $jar
        if ($LASTEXITCODE) { Write-Host 'Preparing the game files failed.' -ForegroundColor Red; exit 1 }
        cargo run --profile fast -p horizon-prepare -- sounds
    }
} finally {
    Pop-Location
}
Write-Host 'Done. Start SteamVR, then run "Play Horizon VR.bat".' -ForegroundColor Green
