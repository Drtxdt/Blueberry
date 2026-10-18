@echo off
setlocal
set "BLUEBERRY_NATIVE_OVERLAY=4"
set "BLUEBERRY_ACTIVE="
set "TERM_PROGRAM="
set "TERM_PROGRAM_VERSION="
set "WT_SESSION="
set "WT_PROFILE_ID="
if exist "%~dp0components\terminal\component-state.json" goto component
if not exist "%~dp0terminal\WindowsTerminal.exe" exit /b 2
if not exist "%~dp0blueberry.exe" exit /b 2
start "" /b "%~dp0terminal\WindowsTerminal.exe" --window new new-tab --inheritEnvironment --startingDirectory "%CD%" "%~dp0blueberry.exe" run --host-mode direct --transport pipe
exit /b %errorlevel%

:component
"%~dp0blueberry.exe" terminal launch
exit /b %errorlevel%
