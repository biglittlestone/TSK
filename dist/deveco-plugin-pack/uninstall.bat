@echo off
setlocal
title TSK x DevEco Code - uninstall
set "BIN=%USERPROFILE%\.tsk\bin"
set "PLUGIN=%USERPROFILE%\.deveco\plugin"

echo.
echo  === TSK x DevEco Code  uninstall ===
echo.

echo  [1/2] Remove TSK binary + PATH entry ...
if exist "%BIN%\tsk.exe" del /Q "%BIN%\tsk.exe"
powershell -NoProfile -Command "$u=[Environment]::GetEnvironmentVariable('Path','User'); $t=$env:USERPROFILE+'\.tsk\bin'; if($u -and ($u -like '*'+$t+'*')){[Environment]::SetEnvironmentVariable('Path',(($u -split ';' | Where-Object { $_ -ne $t -and $_ -ne '' }) -join ';'),'User');'removed from PATH'}else{'not in PATH'}"
if exist "%BIN%" rmdir /Q /S "%BIN%" 2>nul

echo  [2/2] Remove plugin ...
if exist "%PLUGIN%\tsk.ts" del /Q "%PLUGIN%\tsk.ts"

echo  [3/3] Remove '/tsk' command ...
del /Q "%USERPROFILE%\.deveco\command\tsk.js" "%USERPROFILE%\.deveco\command\tsk.md" 2>nul
del /Q "%USERPROFILE%\.config\opencode\command\tsk.js" "%USERPROFILE%\.config\opencode\command\tsk.md" 2>nul

echo.
echo  Uninstalled. Restart DevEco Code if it is running.
echo.
pause
