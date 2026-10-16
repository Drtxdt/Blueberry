@echo off
setlocal
set "BLUEBERRY_NATIVE_OVERLAY=4"
set "BLUEBERRY_ACTIVE="
"%~dp0terminal\WindowsTerminal.exe" --window new new-tab --inheritEnvironment --startingDirectory "%CD%" "%~dp0blueberry.exe" run --host-mode direct --transport pipe
exit /b %errorlevel%
