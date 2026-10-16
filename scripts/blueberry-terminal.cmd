@echo off
setlocal
set "BLUEBERRY_NATIVE_OVERLAY=4"
set "BLUEBERRY_ACTIVE="
if not exist "%~dp0terminal\WindowsTerminal.exe" exit /b 2
if not exist "%~dp0blueberry.exe" exit /b 2
start "" /b "%~dp0terminal\WindowsTerminal.exe" --window new new-tab --inheritEnvironment --startingDirectory "%CD%" "%~dp0blueberry.exe" run --host-mode direct --transport pipe
exit /b %errorlevel%
