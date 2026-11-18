# Horizon VR installer: a small window like Vivecraft's. Copies the prebuilt game beside this
# script to the chosen folder, prepares the game files from your own Minecraft 26.3, and adds a
# desktop shortcut. It finds the Horizon launcher's (or the Minecraft Launcher's) Minecraft 26.3
# and server jar by itself. `-Silent` installs with what it found, without the window. Install
# also copies this installer into the install folder (a Start menu shortcut opens it), where its
# Update button fetches the latest release from GitHub and updates the game and itself.
param([switch]$Silent, [switch]$Repair, [switch]$Uninstall, [switch]$Update, [string]$InstallDir,
    [string]$MinecraftDir, [string]$ServerJar)
Add-Type -AssemblyName System.Windows.Forms, System.Drawing
$here = $PSScriptRoot
$launcher = Join-Path $env:APPDATA 'horizon-launcher'

# A Minecraft folder with 26.3 downloaded: the Horizon launcher's first, then the official one.
function Find-Minecraft {
    foreach ($d in (Join-Path $launcher 'minecraft'), (Join-Path $env:APPDATA '.minecraft')) {
        if (Test-Path (Join-Path $d 'versions\26.3\26.3.jar')) { return $d }
    }
    Join-Path $env:APPDATA '.minecraft'
}

# The pinned 26.3 server jar's SHA-1.
# Beside the installer: in files\ in the unzipped release, at the top in an install folder.
$lock = @((Join-Path $here 'files\research-lock.json'), (Join-Path $here 'research-lock.json')) |
    Where-Object { Test-Path $_ } | Select-Object -First 1
$serverSha = if ($lock) { (Get-Content $lock -Raw | ConvertFrom-Json).minecraft.server_sha1 }
function Is-ServerJar($path) {
    $serverSha -and (Test-Path -LiteralPath $path -PathType Leaf) -and
        (Get-FileHash -LiteralPath $path -Algorithm SHA1).Hash -eq $serverSha
}

# The 26.3 server jar: the one the Horizon launcher downloaded, else a matching one in Downloads.
function Find-ServerJar {
    $candidates = @(Join-Path $launcher 'server-jars\26.3.jar') +
        @(Get-ChildItem (Join-Path $env:USERPROFILE 'Downloads') -Filter '*.jar' -ErrorAction SilentlyContinue |
            Where-Object Length -gt 30MB | ForEach-Object FullName)
    foreach ($jar in $candidates) { if (Is-ServerJar $jar) { return $jar } }
    ''
}

if (-not $InstallDir) {
    # Run from an install folder (its copy of the installer): that folder.
    $InstallDir = if (Test-Path (Join-Path $here 'horizon-client.exe')) { $here } else { Join-Path $env:LOCALAPPDATA 'HorizonVR' }
}

# Where releases come from. HORIZONVR_RELEASES points the tests at a local copy.
$releases = if ($env:HORIZONVR_RELEASES) { $env:HORIZONVR_RELEASES } else {
    'https://api.github.com/repos/v1nx0/horizon-vivecraft-vr/releases/latest'
}
$startMenuLink = Join-Path ([Environment]::GetFolderPath('Programs')) 'Horizon VR Installer.lnk'

# A version file's version (version.txt), or none.
function Read-Version($path) {
    if (Test-Path -LiteralPath $path) { try { [version](Get-Content -LiteralPath $path -Raw).Trim() } catch { $null } }
}

# The latest release: its version and the installer zip's address and SHA-256.
function Get-Latest {
    [Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12
    $r = Invoke-RestMethod -UseBasicParsing -TimeoutSec 15 $releases
    $asset = $r.assets | Where-Object name -eq 'HorizonVR-Installer.zip' | Select-Object -First 1
    if (-not $asset) { throw "The latest release ($($r.tag_name)) has no HorizonVR-Installer.zip." }
    [pscustomobject]@{
        Version = [version]($r.tag_name.TrimStart('v'))
        Url = $asset.browser_download_url
        Sha256 = if ($asset.digest -like 'sha256:*') { $asset.digest.Substring(7) }
    }
}

# Downloads and unzips a release; returns the folder holding its installer and files\.
function Get-Release($latest, $say) {
    $work = Join-Path $env:TEMP "horizonvr-update-$PID"
    if (Test-Path $work) { Remove-Item -Recurse -Force $work }
    New-Item -ItemType Directory $work | Out-Null
    $zip = Join-Path $work 'HorizonVR-Installer.zip'
    & $say "Downloading Horizon VR $($latest.Version)..."
    Invoke-WebRequest -UseBasicParsing $latest.Url -OutFile $zip
    if ($latest.Sha256 -and (Get-FileHash $zip -Algorithm SHA256).Hash -ne $latest.Sha256) {
        throw 'The downloaded release did not match its checksum. Try again.'
    }
    Expand-Archive $zip $work
    $found = Get-ChildItem $work -Recurse -Filter 'HorizonVR-Installer.ps1' | Select-Object -First 1
    if (-not $found -or -not (Test-Path (Join-Path $found.DirectoryName 'files\horizon-client.exe'))) {
        throw 'The downloaded release is missing its game files.'
    }
    $found.DirectoryName
}

# Runs a program, passing each output line to $say, and returns its exit code.
function Run($exe, $arguments, $dir, $say) {
    $out = Join-Path $env:TEMP "horizonvr-$PID.log"
    $p = Start-Process -FilePath $exe -ArgumentList $arguments -WorkingDirectory $dir -NoNewWindow -PassThru `
        -RedirectStandardOutput $out -RedirectStandardError "$out.err"
    # Windows PowerShell only keeps ExitCode when the handle was read while the process ran.
    $null = $p.Handle
    # Each stream keeps its own count of lines shown.
    $shown = @{ $out = 0; "$out.err" = 0 }
    do {
        $done = $p.HasExited
        Start-Sleep -Milliseconds 200
        foreach ($f in $out, "$out.err") {
            $lines = @(Get-Content $f -ErrorAction SilentlyContinue)
            for (; $shown[$f] -lt $lines.Count; $shown[$f]++) { & $say $lines[$shown[$f]] }
        }
    } until ($done)
    $p.WaitForExit()
    Remove-Item $out, "$out.err" -ErrorAction SilentlyContinue
    $p.ExitCode
}

# The whole install. Throws with a message the player can act on.
function Full-Path($path) { [IO.Path]::GetFullPath("$path".Trim('" ')).TrimEnd('\') }

# A running game locks its exe; asking the file works for every spelling of the path (short 8.3
# names, case), where comparing process paths doesn't.
function Assert-Closed($dest) {
    $exe = Join-Path $dest 'horizon-client.exe'
    if (-not (Test-Path -LiteralPath $exe)) { return }
    # Windows keeps the lock a moment after the game exits: give it 5 seconds.
    for ($i = 0; $i -lt 10; $i++) {
        try { [IO.File]::Open($exe, 'Open', 'ReadWrite', 'None').Close(); return } catch { Start-Sleep -Milliseconds 500 }
    }
    throw 'Horizon VR is running from that folder. Close it first.'
}

# What the installer puts in the install folder. Everything else there (saves, options.txt,
# screenshots, resource packs, mods) is the player's and stays.
$installed = 'horizon-client.exe', 'horizon-prepare.exe', 'openxr_loader.dll', 'research-lock.json',
    'LICENSE.txt', 'Play Horizon VR.bat', 'exporters', 'mods\horizon', 'crates\horizon-prepare', '.local',
    '.horizon-cache', 'version.txt', 'HorizonVR-Installer.ps1', 'Horizon VR Installer.bat'

# Removes Horizon VR: its files and the desktop shortcut, keeping the player's worlds and settings.
function Uninstall($dest, $say) {
    $ErrorActionPreference = 'Stop'
    $dest = Full-Path $dest
    if (-not (Test-Path -LiteralPath (Join-Path $dest 'horizon-client.exe')) -or
        -not (Test-Path -LiteralPath (Join-Path $dest 'research-lock.json'))) {
        throw "Horizon VR is not installed in $dest."
    }
    Assert-Closed $dest
    & $say "Removing Horizon VR from $dest..."
    foreach ($name in $installed) {
        $path = Join-Path $dest $name
        if (Test-Path -LiteralPath $path) { Remove-Item -LiteralPath $path -Recurse -Force }
    }
    # The parents of the nested ones, when nothing else is in them.
    foreach ($name in 'mods', 'crates') {
        $path = Join-Path $dest $name
        if ((Test-Path -LiteralPath $path) -and -not (Get-ChildItem -LiteralPath $path -Force)) { Remove-Item -LiteralPath $path }
    }
    foreach ($link in (Join-Path ([Environment]::GetFolderPath('Desktop')) 'Horizon VR.lnk'), $startMenuLink) {
        if (Test-Path -LiteralPath $link) { Remove-Item -LiteralPath $link }
    }
    if (-not (Get-ChildItem -LiteralPath $dest -Force)) {
        Remove-Item -LiteralPath $dest
        & $say 'Horizon VR is uninstalled.'
    } else {
        & $say "Horizon VR is uninstalled. Your worlds and settings are still in $dest."
    }
}

# Reinstalls over a broken install: fresh game files and freshly prepared Minecraft files.
function Repair($dest, $mc, $server, $say) {
    $dest = Full-Path $dest
    Assert-Closed $dest
    $local = Join-Path $dest '.local'
    if (Test-Path -LiteralPath $local) {
        & $say 'Removing the prepared game files...'
        Remove-Item -LiteralPath $local -Recurse -Force
    }
    Install $dest $mc $server $say
}

# Updates an install to the latest release, the installer in it too. False when already latest.
function Update($dest, $mc, $server, $say) {
    $dest = Full-Path $dest
    $have = Read-Version (Join-Path $dest 'version.txt')
    $latest = Get-Latest
    if ($have -and $have -ge $latest.Version) {
        & $say "Horizon VR $have is up to date."
        return $false
    }
    Assert-Closed $dest
    Install $dest $mc $server $say (Get-Release $latest $say)
    $true
}

# `$src`: the folder with the release's installer and files\ (default: beside this script; the
# installer copy in an install folder has none, so it fetches the latest release).
function Install($dest, $mc, $server, $say, $src = $here) {
    $ErrorActionPreference = 'Stop'
    # Full paths without quotes (a command line can leave one after a trailing backslash) or a
    # trailing backslash (it would escape the closing quote of the preparer's arguments).
    $dest = Full-Path $dest
    $mc = Full-Path $mc
    $server = "$server".Trim('" ')
    if (-not (Test-Path (Join-Path $src 'files\horizon-client.exe'))) {
        if (-not (Test-Path (Join-Path $here 'horizon-client.exe'))) {
            throw 'The game files are missing. Unzip the whole HorizonVR-Installer.zip and run the installer from the unzipped folder.'
        }
        $src = Get-Release (Get-Latest) $say
    }
    if (-not (Test-Path (Join-Path $mc 'versions\26.3\26.3.jar'))) {
        throw 'Minecraft 26.3 is not downloaded in that Minecraft folder. Play 26.3 once in the Horizon launcher or the Minecraft Launcher first.'
    }
    if (-not $server -or -not (Is-ServerJar $server)) {
        throw 'Choose the Minecraft 26.3 server jar (download it from minecraft.net). Another version does not work.'
    }
    Assert-Closed $dest

    & $say "Copying Horizon VR to $dest..."
    New-Item -ItemType Directory -Force $dest | Out-Null
    # Replace what an earlier install shipped, so no file a newer version dropped stays behind;
    # .local (the prepared game files) is kept.
    foreach ($item in Get-ChildItem (Join-Path $src 'files')) {
        $old = Join-Path $dest $item.Name
        if (Test-Path -LiteralPath $old) { Remove-Item -LiteralPath $old -Recurse -Force }
        Copy-Item -LiteralPath $item.FullName -Destination $dest -Recurse -Force
    }
    # The installer itself, for its Update, Repair and Uninstall buttons later.
    Copy-Item -LiteralPath (Join-Path $src 'HorizonVR-Installer.ps1') -Destination $dest -Force
    Copy-Item -LiteralPath (Join-Path $src 'Install Horizon VR.bat') -Destination (Join-Path $dest 'Horizon VR Installer.bat') -Force

    # The Minecraft Launcher may rewrite its 26.3 version file; fetch Mojang's own, as the
    # preparer asks, and check it against the pinned SHA-1.
    $sha = (Get-Content (Join-Path $dest 'research-lock.json') -Raw | ConvertFrom-Json).minecraft.manifest_sha1
    $manifest = Join-Path $dest '.local\minecraft\26.3\manifest.json'
    if (-not (Test-Path $manifest) -or (Get-FileHash $manifest -Algorithm SHA1).Hash -ne $sha) {
        & $say 'Downloading the Minecraft 26.3 version file from Mojang...'
        New-Item -ItemType Directory -Force (Split-Path $manifest) | Out-Null
        [Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12
        Invoke-WebRequest -UseBasicParsing "https://piston-meta.mojang.com/v1/packages/$sha/26.3.json" -OutFile $manifest
        if ((Get-FileHash $manifest -Algorithm SHA1).Hash -ne $sha) {
            Remove-Item $manifest
            throw 'The downloaded version file did not match.'
        }
    }

    & $say 'Preparing the game files from your Minecraft (about a minute)...'
    $prepare = Join-Path $dest 'horizon-prepare.exe'
    if ((Run $prepare @('--prepare', '--minecraft-dir', "`"$mc`"", '--server-jar', "`"$server`"") $dest $say) -ne 0) {
        throw 'Preparing the game files failed (see the log).'
    }
    if ((Run $prepare @('sounds', '--minecraft-dir', "`"$mc`"") $dest $say) -ne 0) {
        & $say 'Sounds were not copied; the game runs without them.'
    }

    $shell = New-Object -ComObject WScript.Shell
    $link = $shell.CreateShortcut((Join-Path ([Environment]::GetFolderPath('Desktop')) 'Horizon VR.lnk'))
    $link.TargetPath = Join-Path $dest 'Play Horizon VR.bat'
    $link.WorkingDirectory = $dest
    $link.IconLocation = Join-Path $dest 'horizon-client.exe'
    $link.Save()
    $link = $shell.CreateShortcut($startMenuLink)
    $link.TargetPath = Join-Path $dest 'Horizon VR Installer.bat'
    $link.WorkingDirectory = $dest
    $link.IconLocation = Join-Path $dest 'horizon-client.exe'
    $link.Save()
    & $say 'Done. Start SteamVR, then open "Horizon VR" on your desktop.'
}

# Dot-sourced (by the tests): only define the functions.
if ($MyInvocation.InvocationName -eq '.') { return }

if ($Silent) {
    if (-not $MinecraftDir) { $MinecraftDir = Find-Minecraft }
    if (-not $ServerJar) { $ServerJar = Find-ServerJar }
    $print = { param($t) Write-Host $t }
    try {
        if ($Uninstall) { Uninstall $InstallDir $print }
        elseif ($Update) { $null = Update $InstallDir $MinecraftDir $ServerJar $print }
        elseif ($Repair) { Repair $InstallDir $MinecraftDir $ServerJar $print }
        else { Install $InstallDir $MinecraftDir $ServerJar $print }
        exit 0
    } catch {
        Write-Host "Failed: $_" -ForegroundColor Red
        exit 1
    }
}

[System.Windows.Forms.Application]::EnableVisualStyles()
$form = New-Object System.Windows.Forms.Form
$form.Text = 'Horizon VR Installer'
$form.ClientSize = New-Object System.Drawing.Size(560, 430)
$form.FormBorderStyle = 'FixedDialog'
$form.MaximizeBox = $false
$form.StartPosition = 'CenterScreen'

$status = New-Object System.Windows.Forms.Label
$status.Location = '12,10'; $status.Size = New-Object System.Drawing.Size(536, 20)
$form.Controls.Add($status)
$y = 38
function Row($label, $default, $browse) {
    $l = New-Object System.Windows.Forms.Label
    $l.Text = $label; $l.Location = "12,$script:y"; $l.AutoSize = $true
    $t = New-Object System.Windows.Forms.TextBox
    $t.Text = $default; $t.Location = "12,$($script:y + 20)"; $t.Width = 440
    $b = New-Object System.Windows.Forms.Button
    $b.Text = 'Browse...'; $b.Location = "460,$($script:y + 18)"; $b.Width = 88
    $b.Add_Click({ & $browse $t }.GetNewClosure())
    $form.Controls.AddRange(@($l, $t, $b))
    $script:y += 56
    $t
}
$pickFolder = {
    param($t)
    $d = New-Object System.Windows.Forms.FolderBrowserDialog
    $d.SelectedPath = $t.Text
    if ($d.ShowDialog() -eq 'OK') { $t.Text = $d.SelectedPath }
}
$pickJar = {
    param($t)
    $d = New-Object System.Windows.Forms.OpenFileDialog
    $d.Filter = 'Server jar (*.jar)|*.jar'
    if ($d.ShowDialog() -eq 'OK') { $t.Text = $d.FileName }
}
$installBox = Row 'Install folder' $InstallDir $pickFolder
$mcBox = Row 'Minecraft folder with 26.3 (found automatically)' (Find-Minecraft) $pickFolder
$jarBox = Row 'Minecraft 26.3 server jar (found automatically, or download it from minecraft.net)' (Find-ServerJar) $pickJar

$log = New-Object System.Windows.Forms.TextBox
$log.Multiline = $true; $log.ReadOnly = $true; $log.ScrollBars = 'Vertical'
$log.Location = "12,$y"; $log.Size = New-Object System.Drawing.Size(536, 150)
$form.Controls.Add($log)

$buttons = @()
function Add-Button($text, $x) {
    $b = New-Object System.Windows.Forms.Button
    $b.Text = $text; $b.Location = "$x,$($script:y + 160)"; $b.Size = New-Object System.Drawing.Size(100, 30)
    $form.Controls.Add($b)
    $script:buttons += $b
    $b
}
$uninstallButton = Add-Button 'Uninstall' 12
$updateButton = Add-Button 'Update' 232
$repairButton = Add-Button 'Repair' 340
$button = Add-Button 'Install' 448
$form.AcceptButton = $button

$say = { param($t) $log.AppendText("$t`r`n"); [System.Windows.Forms.Application]::DoEvents() }

# Runs one action with the buttons off, showing its result or error.
function Act($action, $done, $what) {
    $buttons | ForEach-Object { $_.Enabled = $false }
    try {
        & $action
        [System.Windows.Forms.MessageBox]::Show($done) | Out-Null
    } catch {
        & $say "Error: $_"
        [System.Windows.Forms.MessageBox]::Show("$what failed: $_") | Out-Null
    } finally {
        $buttons | ForEach-Object { $_.Enabled = $true }
        Show-Status
    }
}

$button.Add_Click({
    Act { Install $installBox.Text $mcBox.Text $jarBox.Text $say } `
        'Horizon VR is installed. Start SteamVR, then open "Horizon VR" on your desktop.' 'Install'
})
$repairButton.Add_Click({
    Act { Repair $installBox.Text $mcBox.Text $jarBox.Text $say } `
        'Horizon VR is repaired. Start SteamVR, then open "Horizon VR" on your desktop.' 'Repair'
})
$uninstallButton.Add_Click({
    $answer = [System.Windows.Forms.MessageBox]::Show(
        "Uninstall Horizon VR from $($installBox.Text)? Your worlds and settings are kept.",
        'Uninstall Horizon VR', 'YesNo', 'Question')
    if ($answer -eq 'Yes') { Act { Uninstall $installBox.Text $say } 'Horizon VR is uninstalled.' 'Uninstall' }
})

# The installed and latest versions, on the status line; the Update button only when newer.
function Show-Status {
    $have = Read-Version (Join-Path (Full-Path $installBox.Text) 'version.txt')
    $haveText = if ($have) { "Installed: $have" } else { 'Not installed' }
    try {
        $latest = Get-Latest
        $script:updateButton.Enabled = [bool]$have -and $latest.Version -gt $have
        $status.Text = if ($have -and $latest.Version -gt $have) { "$haveText. Update available: $($latest.Version)" }
            elseif ($have) { "$haveText (up to date)" } else { "$haveText. Latest: $($latest.Version)" }
    } catch {
        $script:updateButton.Enabled = [bool]$have
        $status.Text = "$haveText. Couldn't check for updates."
    }
}
$updateButton.Add_Click({
    Act { if (-not (Update $installBox.Text $mcBox.Text $jarBox.Text $say)) { throw 'Horizon VR is already up to date.' } } `
        'Horizon VR is updated. Start SteamVR, then open "Horizon VR" on your desktop.' 'Update'
})
$form.Add_Shown({ Show-Status })

[void]$form.ShowDialog()
