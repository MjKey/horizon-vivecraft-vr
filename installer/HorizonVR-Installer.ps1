# Horizon VR installer: a small window like Vivecraft's. Copies the prebuilt game beside this
# script to the chosen folder, prepares the game files from your own Minecraft 26.3, and adds a
# desktop shortcut.
Add-Type -AssemblyName System.Windows.Forms, System.Drawing
[System.Windows.Forms.Application]::EnableVisualStyles()
$here = $PSScriptRoot

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
$installDir = Row 'Install folder' (Join-Path $env:LOCALAPPDATA 'HorizonVR') $pickFolder
$mcDir = Row 'Minecraft folder (with Minecraft 26.3 installed and played once)' (Join-Path $env:APPDATA '.minecraft') $pickFolder
$jar = Row 'Minecraft 26.3 server jar (download it from minecraft.net)' '' $pickJar

$log = New-Object System.Windows.Forms.TextBox
$log.Multiline = $true; $log.ReadOnly = $true; $log.ScrollBars = 'Vertical'
$log.Location = "12,$y"; $log.Size = New-Object System.Drawing.Size(536, 150)
$form.Controls.Add($log)

$install = New-Object System.Windows.Forms.Button
$install.Text = 'Install'; $install.Location = "448,$($y + 160)"; $install.Size = New-Object System.Drawing.Size(100, 30)
$form.Controls.Add($install)
$form.AcceptButton = $install

function Say($text) { $log.AppendText("$text`r`n"); [System.Windows.Forms.Application]::DoEvents() }

# Runs a program without freezing the window, showing its last output lines.
function Run($exe, $arguments, $dir) {
    $out = Join-Path $env:TEMP "horizonvr-$PID.log"
    $p = Start-Process -FilePath $exe -ArgumentList $arguments -WorkingDirectory $dir -NoNewWindow -PassThru `
        -RedirectStandardOutput $out -RedirectStandardError "$out.err"
    $shown = 0
    while (-not $p.HasExited) {
        Start-Sleep -Milliseconds 200
        $lines = @(Get-Content $out, "$out.err" -ErrorAction SilentlyContinue)
        for (; $shown -lt $lines.Count; $shown++) { Say $lines[$shown] }
        [System.Windows.Forms.Application]::DoEvents()
    }
    $lines = @(Get-Content $out, "$out.err" -ErrorAction SilentlyContinue)
    for (; $shown -lt $lines.Count; $shown++) { Say $lines[$shown] }
    $p.ExitCode
}

$install.Add_Click({
    $dest = $installDir.Text.Trim()
    $server = $jar.Text.Trim('" ')
    if (-not (Test-Path (Join-Path $mcDir.Text 'versions\26.3\26.3.jar'))) {
        [System.Windows.Forms.MessageBox]::Show('Minecraft 26.3 is not downloaded in that Minecraft folder. Play 26.3 once in the Minecraft Launcher first.') | Out-Null
        return
    }
    if (-not (Test-Path $server -PathType Leaf)) {
        [System.Windows.Forms.MessageBox]::Show('Choose the Minecraft 26.3 server jar.') | Out-Null
        return
    }
    $install.Enabled = $false
    try {
        Say "Copying Horizon VR to $dest..."
        New-Item -ItemType Directory -Force $dest | Out-Null
        Get-ChildItem (Join-Path $here 'files') | Copy-Item -Destination $dest -Recurse -Force

        # The Minecraft Launcher may rewrite its 26.3 version file; fetch Mojang's own, as the
        # preparer asks, and check it against the pinned SHA-1.
        $sha = (Get-Content (Join-Path $dest 'research-lock.json') -Raw | ConvertFrom-Json).minecraft.manifest_sha1
        $manifest = Join-Path $dest '.local\minecraft\26.3\manifest.json'
        if (-not (Test-Path $manifest)) {
            Say 'Downloading the Minecraft 26.3 version file from Mojang...'
            New-Item -ItemType Directory -Force (Split-Path $manifest) | Out-Null
            Invoke-WebRequest -UseBasicParsing "https://piston-meta.mojang.com/v1/packages/$sha/26.3.json" -OutFile $manifest
            if ((Get-FileHash $manifest -Algorithm SHA1).Hash -ne $sha) {
                Remove-Item $manifest
                throw 'The downloaded version file did not match.'
            }
        }

        Say 'Preparing the game files from your Minecraft (a few minutes)...'
        $prepare = Join-Path $dest 'horizon-prepare.exe'
        $mc = $mcDir.Text
        if ((Run $prepare @('--prepare', '--minecraft-dir', "`"$mc`"", '--server-jar', "`"$server`"") $dest) -ne 0) {
            throw 'Preparing the game files failed (see above).'
        }
        if ((Run $prepare @('sounds', '--minecraft-dir', "`"$mc`"") $dest) -ne 0) {
            Say 'Sounds were not copied; the game runs without them.'
        }

        $shell = New-Object -ComObject WScript.Shell
        $link = $shell.CreateShortcut((Join-Path ([Environment]::GetFolderPath('Desktop')) 'Horizon VR.lnk'))
        $link.TargetPath = Join-Path $dest 'Play Horizon VR.bat'
        $link.WorkingDirectory = $dest
        $link.IconLocation = Join-Path $dest 'horizon-client.exe'
        $link.Save()
        Say 'Done. Start SteamVR, then open "Horizon VR" on your desktop.'
        [System.Windows.Forms.MessageBox]::Show('Horizon VR is installed. Start SteamVR, then open "Horizon VR" on your desktop.') | Out-Null
    } catch {
        Say "Error: $_"
        [System.Windows.Forms.MessageBox]::Show("Install failed: $_") | Out-Null
    } finally {
        $install.Enabled = $true
    }
})

[void]$form.ShowDialog()
