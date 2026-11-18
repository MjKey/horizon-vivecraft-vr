# Horizon VR installer: a small window like Vivecraft's. Copies the prebuilt game beside this
# script to the chosen folder, prepares the game files from your own Minecraft 26.3, and adds a
# desktop shortcut. It finds the Horizon launcher's (or the Minecraft Launcher's) Minecraft 26.3
# and server jar by itself. `-Silent` installs with what it found, without the window.
param([switch]$Silent, [switch]$Repair, [switch]$Uninstall, [string]$InstallDir, [string]$MinecraftDir, [string]$ServerJar)
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
$serverSha = (Get-Content (Join-Path $here 'files\research-lock.json') -Raw -ErrorAction SilentlyContinue |
    ConvertFrom-Json).minecraft.server_sha1
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

if (-not $InstallDir) { $InstallDir = Join-Path $env:LOCALAPPDATA 'HorizonVR' }

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
    if (Test-Path -LiteralPath $exe) {
        try { [IO.File]::Open($exe, 'Open', 'ReadWrite', 'None').Close() }
        catch { throw 'Horizon VR is running from that folder. Close it first.' }
    }
}

# What the installer puts in the install folder. Everything else there (saves, options.txt,
# screenshots, resource packs, mods) is the player's and stays.
$installed = 'horizon-client.exe', 'horizon-prepare.exe', 'openxr_loader.dll', 'research-lock.json',
    'LICENSE.txt', 'Play Horizon VR.bat', 'exporters', 'mods\horizon', 'crates\horizon-prepare', '.local',
    '.horizon-cache'

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
    $link = Join-Path ([Environment]::GetFolderPath('Desktop')) 'Horizon VR.lnk'
    if (Test-Path -LiteralPath $link) { Remove-Item -LiteralPath $link }
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

function Install($dest, $mc, $server, $say) {
    $ErrorActionPreference = 'Stop'
    # Full paths without quotes (a command line can leave one after a trailing backslash) or a
    # trailing backslash (it would escape the closing quote of the preparer's arguments).
    $dest = Full-Path $dest
    $mc = Full-Path $mc
    $server = "$server".Trim('" ')
    if (-not (Test-Path (Join-Path $here 'files\horizon-client.exe'))) {
        throw 'The game files are missing. Unzip the whole HorizonVR-Installer.zip and run the installer from the unzipped folder.'
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
    foreach ($item in Get-ChildItem (Join-Path $here 'files')) {
        $old = Join-Path $dest $item.Name
        if (Test-Path -LiteralPath $old) { Remove-Item -LiteralPath $old -Recurse -Force }
        Copy-Item -LiteralPath $item.FullName -Destination $dest -Recurse -Force
    }

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
$form.ClientSize = New-Object System.Drawing.Size(560, 400)
$form.FormBorderStyle = 'FixedDialog'
$form.MaximizeBox = $false
$form.StartPosition = 'CenterScreen'

$y = 12
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

[void]$form.ShowDialog()
