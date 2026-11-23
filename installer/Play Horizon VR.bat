@echo off
rem Starts Horizon in VR. Start SteamVR and connect the headset first.
rem It asks first when a newer Horizon VR is out. One line: an update rewrites this file while it runs.
set HORIZON_VR=1
cd /d "%~dp0" & (if exist HorizonVR-Installer.exe start "" /wait HorizonVR-Installer.exe --check) & start "" horizon-client.exe --resources "%~dp0.local\horizon-client\26.3" %* & exit /b
