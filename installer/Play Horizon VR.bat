@echo off
rem Starts Horizon in VR. Start SteamVR and connect the headset first.
set HORIZON_VR=1
cd /d "%~dp0"
start "" horizon-client.exe --resources "%~dp0.local\horizon-client\26.3" %*
