# Horizon VR's installer is HorizonVR-Installer.exe now. This stays for installers 0.3.2 and older:
# their Update looks for this file in the release, and their launcher instance runs it before each
# start. It runs the exe beside it with the same switches and passes its exit code back.
$exe = Join-Path $PSScriptRoot 'HorizonVR-Installer.exe'
if (-not (Test-Path $exe)) { $exe = Join-Path $PSScriptRoot 'files\HorizonVR-Installer.exe' }
$quoted = @($args | ForEach-Object { '"' + "$_".TrimEnd('\') + '"' })
$p = if ($quoted) { Start-Process $exe -ArgumentList $quoted -Wait -PassThru -NoNewWindow } else { Start-Process $exe -Wait -PassThru -NoNewWindow }
exit $p.ExitCode
