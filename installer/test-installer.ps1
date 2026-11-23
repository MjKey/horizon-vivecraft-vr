# Checks each installer bug that was fixed, against an unzipped HorizonVR-Installer.zip.
# Run after build-release.ps1. Uses the Horizon launcher's (or .minecraft's) Minecraft 26.3 and
# server jar on this PC. Installs into a temporary folder; removes the desktop shortcut it makes.
param([string]$Zip = (Join-Path $PSScriptRoot 'out\HorizonVR-Installer.zip'))
$ErrorActionPreference = 'Stop'
$work = Join-Path $env:TEMP "horizonvr-test-$PID"
New-Item -ItemType Directory -Force $work | Out-Null
Expand-Archive $Zip $work
$installer = Join-Path $work 'HorizonVR-Installer\HorizonVR-Installer.exe'
$dest = Join-Path $work 'install dir'
# The shortcuts go to a test folder, not the real Desktop and Start menu.
$env:HORIZONVR_SHORTCUTS = Join-Path $work 'links'
$shortcut = Join-Path $env:HORIZONVR_SHORTCUTS 'Horizon VR.lnk'
$failed = 0
# Leave the player's Horizon launcher alone (its instance is checked by hand).
$env:HORIZONVR_LAUNCHER = Join-Path $work 'no-launcher.exe'
$startMenuLink = Join-Path $env:HORIZONVR_SHORTCUTS 'Horizon VR Installer.lnk'

function Check($name, $ok) {
    if ($ok) { Write-Host "PASS  $name" -ForegroundColor Green }
    else { Write-Host "FAIL  $name" -ForegroundColor Red; $script:failed++ }
}
# Runs an installer exe; each argument quoted (a trailing backslash doubled, as Windows reads it).
function Exe($exe, [string[]]$more) {
    $log = Join-Path $work "out-$([guid]::NewGuid()).txt"
    $quoted = @($more | ForEach-Object { '"' + ($_ -replace '\\$', '\\') + '"' })
    $p = if ($quoted) { Start-Process $exe -ArgumentList $quoted -Wait -PassThru -NoNewWindow -RedirectStandardOutput $log }
        else { Start-Process $exe -Wait -PassThru -NoNewWindow -RedirectStandardOutput $log }
    [pscustomobject]@{ Code = $p.ExitCode; Out = (Get-Content $log -Raw -ErrorAction SilentlyContinue) + '' }
}
function Silent([string[]]$more) { Exe $installer (@('-Silent') + $more) }

# The logic (versions, paths, Run, the launcher base fallback, the server jar search): unit tests.
$unit = & cargo test --release --quiet --manifest-path (Join-Path $PSScriptRoot 'app\Cargo.toml') 2>&1 | Out-String
Check 'the installer''s unit tests pass' ($LASTEXITCODE -eq 0)
if ($LASTEXITCODE) { Write-Host $unit }

$launcher = Join-Path $env:APPDATA 'horizon-launcher'
$mc = @((Join-Path $launcher 'minecraft'), (Join-Path $env:APPDATA '.minecraft')) |
    Where-Object { Test-Path (Join-Path $_ 'versions\26.3\26.3.jar') } | Select-Object -First 1
$sha = (Get-Content (Join-Path $work 'HorizonVR-Installer\files\research-lock.json') -Raw | ConvertFrom-Json).minecraft.server_sha1
$jar = @(Join-Path $launcher 'server-jars\26.3.jar') + @(Get-ChildItem (Join-Path $launcher 'versions\*\.local\minecraft\26.3\jars\server.jar') -ErrorAction SilentlyContinue | ForEach-Object FullName) |
    Where-Object { (Test-Path $_) -and (Get-FileHash $_ -Algorithm SHA1).Hash -eq $sha } | Select-Object -First 1
Check 'finds Minecraft 26.3 and the server jar' ($jar -and $mc)
$fakeHome = Join-Path $work 'home'
New-Item -ItemType Directory -Force (Join-Path $fakeHome 'Downloads') | Out-Null
$f = [IO.File]::Create((Join-Path $fakeHome 'Downloads\server-1.21.4.jar')); $f.SetLength(31MB); $f.Close()

# JDK check in install.ps1: the major version read from `javac -version`.
$major = { param($v) [int]((($v -replace '[^0-9.]', '').Split('.'))[0]) }
Check 'JDK 17 is refused, 25 accepted' (((& $major 'javac 17.0.2') -lt 25) -and ((& $major 'javac 25.0.1') -ge 25))

# A wrong server jar is refused.
$r = Silent @('-InstallDir', $dest, '-ServerJar', (Join-Path $fakeHome 'Downloads\server-1.21.4.jar'))
Check 'a wrong server jar is refused' ($r.Code -eq 1 -and $r.Out -match 'server jar')

# The small installer alone downloads the game; offline it says so.
$alone = Join-Path $work 'alone'
New-Item -ItemType Directory -Force $alone | Out-Null
Copy-Item $installer $alone
$env:HORIZONVR_RELEASES = 'http://127.0.0.1:1/latest.json'
$r = Exe (Join-Path $alone 'HorizonVR-Installer.exe') @('-Silent', '-InstallDir', $dest, '-ServerJar', $jar)
Remove-Item Env:HORIZONVR_RELEASES
Check 'the small installer offline says so' ($r.Code -eq 1 -and $r.Out -match 'internet')

# A file errors stops the install (the install folder is a file here).
$asFile = Join-Path $work 'a file'
Set-Content $asFile 'x'
$r = Silent @('-InstallDir', $asFile)
Check 'a copy error stops the install' ($r.Code -eq 1 -and $r.Out -notmatch 'Done\.')

# A full install, as the window calls it, with trailing backslashes on both folders (a command
# line can't pass those: Windows reads a backslash before a quote as part of the argument).
$r = Silent @('-InstallDir', "$dest\", '-MinecraftDir', "$mc\", '-ServerJar', $jar)
$ok = $r.Code -eq 0
if (-not $ok) { $r.Out -split "`n" | Select-Object -Last 3 | Write-Host }
$installLog = $r.Out
Check 'installs with trailing backslashes' ($ok -and (Test-Path (Join-Path $dest '.local\horizon-client\26.3')))
Check 'uses the shipped exporters (compiles nothing)' ($installLog -notmatch 'ompil')
# The shortcut may spell the folder with its long name where $dest has the short (8.3) one.
$target = (New-Object -ComObject WScript.Shell).CreateShortcut($shortcut).TargetPath
Check 'makes the desktop shortcut' ((Test-Path -LiteralPath $target) -and $target -like '*\install dir\Play Horizon VR.bat')

# Reinstall: a file the release no longer ships is removed; the prepared game files stay.
Set-Content (Join-Path $dest 'exporters\stale.txt') 'old'
# An older install (the same version is skipped as installed).
Remove-Item (Join-Path $dest 'version.txt')
$r = Silent @('-InstallDir', $dest)
Check 'reinstall removes old files' ($r.Code -eq 0 -and -not (Test-Path (Join-Path $dest 'exporters\stale.txt')))
Check 'reinstall keeps the prepared files' (Test-Path (Join-Path $dest '.local\horizon-client\26.3'))

# The running game is noticed, even with a trailing backslash on the folder.
$game = Start-Process (Join-Path $dest 'horizon-client.exe') -ArgumentList '--resources', "`"$dest\.local\horizon-client\26.3`"" -WorkingDirectory $dest -PassThru
Start-Sleep 5
Remove-Item (Join-Path $dest 'version.txt')
$r = Silent @('-InstallDir', "$dest\")
Check 'refuses while the game runs' ($r.Code -eq 1 -and $r.Out -match 'running')
Stop-Process $game -ErrorAction SilentlyContinue
$game.WaitForExit()

# The window: click Install and wait for the "installed" message.
Add-Type -AssemblyName UIAutomationClient, UIAutomationTypes
$gui = Start-Process $installer -ArgumentList '-InstallDir', "`"$dest`"" -PassThru
$root = [Windows.Automation.AutomationElement]::RootElement
$byPid = [Windows.Automation.PropertyCondition]::new([Windows.Automation.AutomationElement]::ProcessIdProperty, $gui.Id)
$deadline = (Get-Date).AddSeconds(120)
do { Start-Sleep -Milliseconds 500; $window = $root.FindFirst('Children', $byPid) } until ($window -or (Get-Date) -gt $deadline)
$byName = [Windows.Automation.PropertyCondition]::new([Windows.Automation.AutomationElement]::NameProperty, 'Install')
$button = if ($window) { $window.FindFirst('Descendants', $byName) }
# BM_CLICK posted to the button (sending would wait for the whole install).
Add-Type -Namespace Win32 -Name User -MemberDefinition '[DllImport("user32.dll")] public static extern bool PostMessage(IntPtr h, uint m, IntPtr w, IntPtr l);'
if ($button) { [void][Win32.User]::PostMessage([IntPtr]$button.Current.NativeWindowHandle, 0xF5, [IntPtr]::Zero, [IntPtr]::Zero) }
$message = $null
$deadline = (Get-Date).AddMinutes(5)
do {
    Start-Sleep 1
    # The result is a message box: one more top-level window of the installer, beside the main one.
    $message = $root.FindAll('Children', $byPid) | ForEach-Object {
        $_.FindAll('Descendants', [Windows.Automation.Condition]::TrueCondition) | ForEach-Object { $_.Current.Name }
    } | Where-Object { $_ -match 'is installed|failed:' } | Select-Object -First 1
} until ($message -or (Get-Date) -gt $deadline)
Check "the window's Install button installs (window $([bool]$window), button $([bool]$button): $message)" ($message -match 'is installed')
Stop-Process $gui -ErrorAction SilentlyContinue

# Repair: a broken install (game deleted, prepared files damaged) comes back whole.
Remove-Item (Join-Path $dest 'horizon-client.exe')
Set-Content (Join-Path $dest '.local\horizon-client\26.3\broken.txt') 'x'
$r = Silent @('-Repair', '-InstallDir', $dest)
Check 'repair restores a broken install' ($r.Code -eq 0 -and (Test-Path (Join-Path $dest 'horizon-client.exe')) -and
    -not (Test-Path (Join-Path $dest '.local\horizon-client\26.3\broken.txt')) -and (Test-Path (Join-Path $dest '.local\horizon-client\26.3')))

Check 'install copies the installer and its version' ((Test-Path (Join-Path $dest 'HorizonVR-Installer.exe')) -and
    (Test-Path (Join-Path $dest 'version.txt')))
# Older installers' Update takes HorizonVR-Installer.zip: its layout stays theirs.
Check 'the old installers'' zip keeps its layout' ((Test-Path (Join-Path $work 'HorizonVR-Installer\HorizonVR-Installer.ps1')) -and
    (Test-Path (Join-Path $work 'HorizonVR-Installer\files\horizon-client.exe')))

# Update: a fake newer release (this payload, as 9.9.9) served the way GitHub's API gives it.
$serve = Join-Path $work 'serve'
$newer = Join-Path $work 'newer'
New-Item -ItemType Directory -Force $serve | Out-Null
Copy-Item -Recurse (Join-Path $work 'HorizonVR-Installer\files') (Join-Path $newer 'files')
Set-Content (Join-Path $newer 'files\version.txt') '9.9.9'
$payload = Join-Path $serve 'HorizonVR-Payload.zip'
Compress-Archive (Join-Path $newer 'files') $payload
$payloadSha = (Get-FileHash $payload -Algorithm SHA256).Hash.ToLower()
Set-Content "$payload.sha256" "$payloadSha  HorizonVR-Payload.zip"
$port = 18765
# $digest: GitHub's (none: only the .sha256 file says it); $sidecar: that file is listed.
function Write-Release($digest, [bool]$sidecar) {
    $a = @{ name = 'HorizonVR-Payload.zip'; browser_download_url = "http://127.0.0.1:$port/HorizonVR-Payload.zip" }
    if ($digest) { $a.digest = "sha256:$digest" }
    $assets = @($a)
    if ($sidecar) { $assets += @{ name = 'HorizonVR-Payload.zip.sha256'; browser_download_url = "http://127.0.0.1:$port/HorizonVR-Payload.zip.sha256" } }
    @{ tag_name = 'v9.9.9'; assets = $assets } | ConvertTo-Json -Depth 4 | Set-Content (Join-Path $serve 'latest.json')
}
Write-Release ('0' * 64) $false
$server = Start-Process python -ArgumentList '-m', 'http.server', $port, '--bind', '127.0.0.1' -WorkingDirectory $serve -PassThru -WindowStyle Hidden
Start-Sleep 2
$env:HORIZONVR_RELEASES = "http://127.0.0.1:$port/latest.json"
$installedCopy = Join-Path $dest 'HorizonVR-Installer.exe'
function FromInstall([string[]]$more) { Exe $installedCopy (@('-Silent') + $more) }
$r = FromInstall @('-Update')
Check 'update refuses a download with the wrong checksum' ($r.Code -eq 1 -and $r.Out -match 'checksum' -and
    (Get-Content (Join-Path $dest 'version.txt')).Trim() -ne '9.9.9')
Write-Release $null $false
$r = FromInstall @('-Update')
Check 'update refuses a release without a checksum' ($r.Code -eq 1 -and $r.Out -match 'checksum' -and
    (Get-Content (Join-Path $dest 'version.txt')).Trim() -ne '9.9.9')
Write-Release $null $true
$r = FromInstall @('-Update')
Check 'update installs the newer release' ($r.Code -eq 0 -and (Get-Content (Join-Path $dest 'version.txt')).Trim() -eq '9.9.9')
$r = FromInstall @('-Update')
Check "update says when it is up to date ($($r.Code): $($r.Out.Trim()))" ($r.Code -eq 0 -and $r.Out -match 'up to date')
$r = FromInstall @('-Check')
Check 'the start check stays quiet when up to date' ($r.Code -eq 0 -and -not $r.Out.Trim())
$env:HORIZONVR_RELEASES = 'http://127.0.0.1:1/latest.json'
$r = FromInstall @('-Check')
Check 'the start check stays quiet offline' ($r.Code -eq 0)
$env:HORIZONVR_RELEASES = "http://127.0.0.1:$port/latest.json"
# The installer copy in the install folder installs (repairs) from the latest release by itself.
Remove-Item (Join-Path $dest 'horizon-client.exe')
$r = FromInstall @()
Check 'the installed installer reinstalls from the latest release' ($r.Code -eq 0 -and (Test-Path (Join-Path $dest 'horizon-client.exe')))
# The small installer on its own (as players download it) installs from the payload.
Remove-Item (Join-Path $dest 'horizon-client.exe')
$r = Exe (Join-Path $alone 'HorizonVR-Installer.exe') @('-Silent', '-InstallDir', $dest)
Check 'the small installer alone downloads and installs the game' ($r.Code -eq 0 -and (Test-Path (Join-Path $dest 'horizon-client.exe')) -and $r.Out -match 'Downloading')
Stop-Process $server -ErrorAction SilentlyContinue
Remove-Item Env:HORIZONVR_RELEASES

# Uninstall refuses a folder without Horizon VR in it.
$r = Silent @('-Uninstall', '-InstallDir', $alone)
Check 'uninstall refuses a folder without Horizon VR' ($r.Code -eq 1 -and (Test-Path (Join-Path $alone 'HorizonVR-Installer.exe')))
# Installers 0.3.2 and older run the release's HorizonVR-Installer.ps1: it runs the exe.
$legacy = & powershell -NoProfile -ExecutionPolicy Bypass -File (Join-Path $work 'HorizonVR-Installer\HorizonVR-Installer.ps1') -Silent -Uninstall -InstallDir $alone 2>&1 | Out-String
Check "the old installers' .ps1 runs the exe ($LASTEXITCODE)" ($LASTEXITCODE -eq 1 -and $legacy -match 'not installed')

# Uninstall: the game, the prepared files and the shortcut go; the player's worlds and settings stay.
New-Item -ItemType Directory -Force (Join-Path $dest 'saves\My World') | Out-Null
Set-Content (Join-Path $dest 'saves\My World\level.dat') 'world'
Set-Content (Join-Path $dest 'options.txt') 'fov:90'
$r = Silent @('-Uninstall', '-InstallDir', $dest)
# The game itself also writes settings here (config, horizon-graphics.txt): the player's too.
$left = @(Get-ChildItem $dest -Force | ForEach-Object Name | Sort-Object)
# Everything the release ships (so a newly shipped file can't be left behind unnoticed).
$shipped = @(Get-ChildItem (Join-Path $work 'HorizonVR-Installer\files') -Force | ForEach-Object Name) +
    '.local', '.horizon-cache'
Check "uninstall keeps only the player's files ($($left -join ','))" ($r.Code -eq 0 -and
    'options.txt' -in $left -and 'saves' -in $left -and -not ($left | Where-Object { $_ -in $shipped }))
Check 'uninstall keeps the world' ((Get-Content (Join-Path $dest 'saves\My World\level.dat')) -eq 'world')
Check 'uninstall removes the shortcuts' (-not (Test-Path $shortcut) -and -not (Test-Path $startMenuLink))

# Uninstall of a folder with nothing of the player's removes the folder too.
$r = Silent @('-InstallDir', $dest)
Get-ChildItem $dest -Force | Where-Object Name -in $left | Remove-Item -Recurse -Force
$r = Silent @('-Uninstall', '-InstallDir', $dest)
Check 'uninstall removes an empty install folder' ($r.Code -eq 0 -and -not (Test-Path $dest))

Remove-Item -Recurse -Force $work -ErrorAction SilentlyContinue
Remove-Item Env:HORIZONVR_LAUNCHER, Env:HORIZONVR_RELEASES, Env:HORIZONVR_SHORTCUTS -ErrorAction SilentlyContinue
if ($failed) { Write-Host "$failed check(s) failed" -ForegroundColor Red; exit 1 }
Write-Host 'All checks passed' -ForegroundColor Green


