# Horizon VR installer: a small window like Vivecraft's. Copies the prebuilt game beside this
# script to the chosen folder, prepares the game files from your own Minecraft 26.3, and adds a
# desktop shortcut. It finds the Horizon launcher's (or the Minecraft Launcher's) Minecraft 26.3
# and server jar by itself. `-Silent` installs with what it found, without the window.
param([switch]$Silent, [string]$InstallDir)
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
function Install($dest, $mc, $server, $say) {
    $ErrorActionPreference = 'Stop'
    if (-not (Test-Path (Join-Path $here 'files\horizon-client.exe'))) {
        throw 'The game files are missing. Unzip the whole HorizonVR-Installer.zip and run the installer from the unzipped folder.'
    }
    if (-not (Test-Path (Join-Path $mc 'versions\26.3\26.3.jar'))) {
        throw 'Minecraft 26.3 is not downloaded in that Minecraft folder. Play 26.3 once in the Horizon launcher or the Minecraft Launcher first.'
    }
    if (-not $server -or -not (Is-ServerJar $server)) {
        throw 'Choose the Minecraft 26.3 server jar (download it from minecraft.net). Another version does not work.'
    }
    # No trailing backslash: it would escape the closing quote of a program argument.
    $dest = [IO.Path]::GetFullPath($dest).TrimEnd('\')
    $mc = [IO.Path]::GetFullPath($mc).TrimEnd('\')
    if ((Get-Process horizon-client -ErrorAction SilentlyContinue | Where-Object { $_.Path -and (Split-Path $_.Path) -eq $dest })) {
        throw 'Horizon VR is running from that folder. Close it first.'
    }

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

if ($Silent) {
    try {
        Install $InstallDir (Find-Minecraft) (Find-ServerJar) { param($t) Write-Host $t }
        exit 0
    } catch {
        Write-Host "Install failed: $_" -ForegroundColor Red
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

$button = New-Object System.Windows.Forms.Button
$button.Text = 'Install'; $button.Location = "448,$($y + 160)"; $button.Size = New-Object System.Drawing.Size(100, 30)
$form.Controls.Add($button)
$form.AcceptButton = $button

$say = { param($t) $log.AppendText("$t`r`n"); [System.Windows.Forms.Application]::DoEvents() }

$button.Add_Click({
    $button.Enabled = $false
    try {
        Install $installBox.Text.Trim() $mcBox.Text.Trim() $jarBox.Text.Trim('" ') $say
        [System.Windows.Forms.MessageBox]::Show('Horizon VR is installed. Start SteamVR, then open "Horizon VR" on your desktop.') | Out-Null
    } catch {
        & $say "Error: $_"
        [System.Windows.Forms.MessageBox]::Show("Install failed: $_") | Out-Null
    } finally {
        $button.Enabled = $true
    }
})

[void]$form.ShowDialog()
