# Checks each installer bug that was fixed, against an unzipped HorizonVR-Installer.zip.
# Run after build-release.ps1. Uses the Horizon launcher's (or .minecraft's) Minecraft 26.3 and
# server jar on this PC. Installs into a temporary folder; removes the desktop shortcut it makes.
param([string]$Zip = (Join-Path $PSScriptRoot 'out\HorizonVR-Installer.zip'))
$ErrorActionPreference = 'Stop'
$work = Join-Path $env:TEMP "horizonvr-test-$PID"
New-Item -ItemType Directory -Force $work | Out-Null
Expand-Archive $Zip $work
$installer = Join-Path $work 'HorizonVR-Installer\HorizonVR-Installer.ps1'
$dest = Join-Path $work 'install dir'
$shortcut = Join-Path ([Environment]::GetFolderPath('Desktop')) 'Horizon VR.lnk'
$hadShortcut = Test-Path $shortcut
$hadStartMenu = Test-Path (Join-Path ([Environment]::GetFolderPath('Programs')) 'Horizon VR Installer.lnk')
$failed = 0
# Leave the player's Horizon launcher alone (its instance is checked by hand).
$env:HORIZONVR_LAUNCHER = Join-Path $work 'no-launcher.exe'

function Check($name, $ok) {
    if ($ok) { Write-Host "PASS  $name" -ForegroundColor Green }
    else { Write-Host "FAIL  $name" -ForegroundColor Red; $script:failed++ }
}
function Silent([string[]]$more) {
    $out = & powershell -NoProfile -ExecutionPolicy Bypass -File $installer -Silent @more 2>&1 | Out-String
    [pscustomobject]@{ Code = $LASTEXITCODE; Out = $out }
}

. $installer
$mc = Find-Minecraft
$jar = Find-ServerJar
Check 'finds Minecraft 26.3 and the server jar' ($jar -and (Test-Path (Join-Path $mc 'versions\26.3\26.3.jar')))

# Run: the exit code survives (Windows PowerShell drops it unless the handle is read).
Check 'Run returns the exit code' ((Run 'cmd.exe' @('/c', 'exit 3') $work { }) -eq 3)

# Run: stdout and stderr lines each shown once, even when they interleave.
$seen = [Collections.Generic.List[string]]::new()
$null = Run 'cmd.exe' @('/c', '"echo out1& echo err1 1>&2& ping -n 2 127.0.0.1 >nul& echo out2& echo err2 1>&2"') $work { param($t) $seen.Add($t.Trim()) }
Check 'Run shows every output line once' ((($seen | Sort-Object) -join ',') -eq 'err1,err2,out1,out2')

# Find-ServerJar: another jar in Downloads is not taken for the 26.3 server jar.
$fakeHome = Join-Path $work 'home'
New-Item -ItemType Directory -Force (Join-Path $fakeHome 'Downloads') | Out-Null
$f = [IO.File]::Create((Join-Path $fakeHome 'Downloads\server-1.21.4.jar')); $f.SetLength(31MB); $f.Close()
$realLauncher, $realHome = $launcher, $env:USERPROFILE
$launcher, $env:USERPROFILE = (Join-Path $work 'no-launcher'), $fakeHome
Check 'another version''s server jar is not found' ((Find-ServerJar) -eq '')
$launcher, $env:USERPROFILE = $realLauncher, $realHome

# JDK check in install.ps1: the major version read from `javac -version`.
$major = { param($v) [int]((($v -replace '[^0-9.]', '').Split('.'))[0]) }
Check 'JDK 17 is refused, 25 accepted' (((& $major 'javac 17.0.2') -lt 25) -and ((& $major 'javac 25.0.1') -ge 25))

# A wrong server jar is refused.
$r = Silent @('-InstallDir', $dest, '-ServerJar', (Join-Path $fakeHome 'Downloads\server-1.21.4.jar'))
Check 'a wrong server jar is refused' ($r.Code -eq 1 -and $r.Out -match 'server jar')

# The installer alone, without the files beside it, says to unzip.
$alone = Join-Path $work 'alone'
New-Item -ItemType Directory -Force $alone | Out-Null
Copy-Item $installer $alone
$r = & powershell -NoProfile -ExecutionPolicy Bypass -File (Join-Path $alone 'HorizonVR-Installer.ps1') -Silent -InstallDir $dest -ServerJar $jar 2>&1 | Out-String
Check 'missing game files say to unzip' ($LASTEXITCODE -eq 1 -and $r -match 'Unzip')

# A file errors stops the install (the install folder is a file here).
$asFile = Join-Path $work 'a file'
Set-Content $asFile 'x'
$r = Silent @('-InstallDir', $asFile)
Check 'a copy error stops the install' ($r.Code -eq 1 -and $r.Out -notmatch 'Done\.')

# A full install, as the window calls it, with trailing backslashes on both folders (a command
# line can't pass those: Windows reads a backslash before a quote as part of the argument).
$installLog = [Collections.Generic.List[string]]::new()
$ok = try { Install "$dest\" "$mc\" $jar { param($t) $installLog.Add($t) }; $true } catch { $installLog.Add("$_"); $false }
if (-not $ok) { $installLog | Select-Object -Last 3 | Write-Host }
Check 'installs with trailing backslashes' ($ok -and (Test-Path (Join-Path $dest '.local\horizon-client\26.3')))
Check 'uses the shipped exporters (compiles nothing)' (-not ($installLog -match 'ompil'))
# The shortcut may spell the folder with its long name where $dest has the short (8.3) one.
$target = (New-Object -ComObject WScript.Shell).CreateShortcut($shortcut).TargetPath
Check 'makes the desktop shortcut' ((Test-Path -LiteralPath $target) -and $target -like '*\install dir\Play Horizon VR.bat')

# Reinstall: a file the release no longer ships is removed; the prepared game files stay.
Set-Content (Join-Path $dest 'exporters\stale.txt') 'old'
$r = Silent @('-InstallDir', $dest)
Check 'reinstall removes old files' ($r.Code -eq 0 -and -not (Test-Path (Join-Path $dest 'exporters\stale.txt')))
Check 'reinstall keeps the prepared files' (Test-Path (Join-Path $dest '.local\horizon-client\26.3'))

# The running game is noticed, even with a trailing backslash on the folder.
$game = Start-Process (Join-Path $dest 'horizon-client.exe') -ArgumentList '--resources', "`"$dest\.local\horizon-client\26.3`"" -WorkingDirectory $dest -PassThru
Start-Sleep 5
$r = Silent @('-InstallDir', "$dest\")
Check 'refuses while the game runs' ($r.Code -eq 1 -and $r.Out -match 'running')
Stop-Process $game -ErrorAction SilentlyContinue
$game.WaitForExit()

# The window: click Install and wait for the "installed" message.
Add-Type -AssemblyName UIAutomationClient, UIAutomationTypes
$gui = Start-Process powershell -ArgumentList '-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', "`"$installer`"", '-InstallDir', "`"$dest`"" -PassThru
$root = [Windows.Automation.AutomationElement]::RootElement
$byPid = [Windows.Automation.PropertyCondition]::new([Windows.Automation.AutomationElement]::ProcessIdProperty, $gui.Id)
$deadline = (Get-Date).AddSeconds(120)
do { Start-Sleep -Milliseconds 500; $window = $root.FindFirst('Children', $byPid) } until ($window -or (Get-Date) -gt $deadline)
$byName = [Windows.Automation.PropertyCondition]::new([Windows.Automation.AutomationElement]::NameProperty, 'Install')
$button = if ($window) { $window.FindFirst('Descendants', $byName) }
# WinForms buttons show up as panes without an invoke pattern: post BM_CLICK (posted, as sending
# would wait for the whole install).
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

# Update: a fake newer release (this zip, as 9.9.9) served the way GitHub's API gives it.
Check 'install copies the installer and its version' ((Test-Path (Join-Path $dest 'HorizonVR-Installer.ps1')) -and
    (Test-Path (Join-Path $dest 'Horizon VR Installer.bat')) -and (Test-Path (Join-Path $dest 'version.txt')))
$serve = Join-Path $work 'serve'
$newer = Join-Path $work 'newer'
New-Item -ItemType Directory -Force $serve | Out-Null
Expand-Archive $Zip $newer
Set-Content (Join-Path $newer 'HorizonVR-Installer\files\version.txt') '9.9.9'
Compress-Archive (Join-Path $newer 'HorizonVR-Installer') (Join-Path $serve 'HorizonVR-Installer.zip')
$port = 18765
function Write-Release($sha) {
    @{ tag_name = 'v9.9.9'; assets = @(@{ name = 'HorizonVR-Installer.zip'
        browser_download_url = "http://127.0.0.1:$port/HorizonVR-Installer.zip"; digest = "sha256:$sha" }) } |
        ConvertTo-Json -Depth 4 | Set-Content (Join-Path $serve 'latest.json')
}
Write-Release ('0' * 64)
$server = Start-Process python -ArgumentList '-m', 'http.server', $port, '--bind', '127.0.0.1' -WorkingDirectory $serve -PassThru -WindowStyle Hidden
Start-Sleep 2
$env:HORIZONVR_RELEASES = "http://127.0.0.1:$port/latest.json"
$installedCopy = Join-Path $dest 'HorizonVR-Installer.ps1'
function FromInstall([string[]]$more) {
    $out = & powershell -NoProfile -ExecutionPolicy Bypass -File $installedCopy -Silent @more 2>&1 | Out-String
    [pscustomobject]@{ Code = $LASTEXITCODE; Out = $out }
}
$r = FromInstall @('-Update')
Check 'update refuses a download with the wrong checksum' ($r.Code -eq 1 -and $r.Out -match 'checksum' -and
    (Get-Content (Join-Path $dest 'version.txt')).Trim() -ne '9.9.9')
Write-Release (Get-FileHash (Join-Path $serve 'HorizonVR-Installer.zip') -Algorithm SHA256).Hash
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
Remove-Item (Join-Path $dest 'horizon-prepare.exe')
$r = FromInstall @()
Check 'the installed installer reinstalls from the latest release' ($r.Code -eq 0 -and (Test-Path (Join-Path $dest 'horizon-prepare.exe')))
Stop-Process $server -ErrorAction SilentlyContinue
Remove-Item Env:HORIZONVR_RELEASES

# Uninstall refuses a folder without Horizon VR in it.
$r = Silent @('-Uninstall', '-InstallDir', $alone)
Check 'uninstall refuses a folder without Horizon VR' ($r.Code -eq 1 -and (Test-Path (Join-Path $alone 'HorizonVR-Installer.ps1')))

# Uninstall: the game, the prepared files and the shortcut go; the player's worlds and settings stay.
New-Item -ItemType Directory -Force (Join-Path $dest 'saves\My World') | Out-Null
Set-Content (Join-Path $dest 'saves\My World\level.dat') 'world'
Set-Content (Join-Path $dest 'options.txt') 'fov:90'
$r = Silent @('-Uninstall', '-InstallDir', $dest)
# The game itself also writes settings here (config, horizon-graphics.txt): the player's too.
$left = @(Get-ChildItem $dest -Force | ForEach-Object Name | Sort-Object)
# Everything the release ships (so a newly shipped file can't be left behind unnoticed).
$shipped = @(Get-ChildItem (Join-Path $work 'HorizonVR-Installer\files') -Force | ForEach-Object Name) +
    '.local', '.horizon-cache', 'HorizonVR-Installer.ps1', 'Horizon VR Installer.bat'
Check "uninstall keeps only the player's files ($($left -join ','))" ($r.Code -eq 0 -and
    'options.txt' -in $left -and 'saves' -in $left -and -not ($left | Where-Object { $_ -in $shipped }))
Check 'uninstall keeps the world' ((Get-Content (Join-Path $dest 'saves\My World\level.dat')) -eq 'world')
Check 'uninstall removes the shortcuts' (-not (Test-Path $shortcut) -and -not (Test-Path $startMenuLink))

# Uninstall of a folder with nothing of the player's removes the folder too.
$r = Silent @('-InstallDir', $dest)
Get-ChildItem $dest -Force | Where-Object Name -in $left | Remove-Item -Recurse -Force
$r = Silent @('-Uninstall', '-InstallDir', $dest)
Check 'uninstall removes an empty install folder' ($r.Code -eq 0 -and -not (Test-Path $dest))

if (-not $hadShortcut) { Remove-Item $shortcut -ErrorAction SilentlyContinue }
if (-not $hadStartMenu) { Remove-Item $startMenuLink -ErrorAction SilentlyContinue }
Remove-Item -Recurse -Force $work -ErrorAction SilentlyContinue
Remove-Item Env:HORIZONVR_LAUNCHER, Env:HORIZONVR_RELEASES -ErrorAction SilentlyContinue
if ($failed) { Write-Host "$failed check(s) failed" -ForegroundColor Red; exit 1 }
Write-Host 'All checks passed' -ForegroundColor Green


