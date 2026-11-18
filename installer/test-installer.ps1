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
$failed = 0

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
$deadline = (Get-Date).AddSeconds(30)
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
    $dialogs = if ($window) { $window.FindAll('Descendants', [Windows.Automation.PropertyCondition]::new([Windows.Automation.AutomationElement]::ControlTypeProperty, [Windows.Automation.ControlType]::Text)) }
    $message = $dialogs | ForEach-Object { $_.Current.Name } | Where-Object { $_ -match 'installed|failed' } | Select-Object -First 1
    if (-not $message) {
        $popup = $root.FindFirst('Children', [Windows.Automation.AndCondition]::new($byPid, [Windows.Automation.PropertyCondition]::new([Windows.Automation.AutomationElement]::ControlTypeProperty, [Windows.Automation.ControlType]::Window)))
        if ($popup -and $popup.Current.Name -eq '') {
            $message = $popup.FindAll('Descendants', [Windows.Automation.Condition]::TrueCondition) | ForEach-Object { $_.Current.Name } | Where-Object { $_ -match 'installed|failed' } | Select-Object -First 1
        }
    }
} until ($message -or (Get-Date) -gt $deadline)
Check "the window's Install button installs ($message)" ($message -match 'is installed')
Stop-Process $gui -ErrorAction SilentlyContinue

if (-not $hadShortcut) { Remove-Item $shortcut -ErrorAction SilentlyContinue }
Remove-Item -Recurse -Force $work -ErrorAction SilentlyContinue
if ($failed) { Write-Host "$failed check(s) failed" -ForegroundColor Red; exit 1 }
Write-Host 'All checks passed' -ForegroundColor Green

