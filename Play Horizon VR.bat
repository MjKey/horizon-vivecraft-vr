@echo off
rem Starts our Horizon build in VR. Start SteamVR and connect the Quest first.
set HORIZON_VR=1
cd /d "%~dp0horizon\target\fast"
start "" horizon-client.exe --resources "%~dp0horizon\.local\horizon-client\26.3" %*
