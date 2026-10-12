@echo off
setlocal
title TSK x DevEco Code - one-click install
set "SCRIPT_DIR=%~dp0"
set "BIN=%USERPROFILE%\.tsk\bin"
set "PLUGIN=%USERPROFILE%\.deveco\plugin"

echo.
echo  === TSK x DevEco Code  one-click install ===
echo.

echo  [1/3] Install TSK core (tsk.exe) ...
if not exist "%BIN%" mkdir "%BIN%"
copy /Y "%SCRIPT_DIR%tsk.exe" "%BIN%\tsk.exe" >nul
if errorlevel 1 goto :fail_core

echo  [2/3] Add tsk to user PATH (takes effect in new terminals) ...
powershell -NoProfile -Command "$u=[Environment]::GetEnvironmentVariable('Path','User'); $t=$env:USERPROFILE+'\.tsk\bin'; if($u -and ($u -like '*'+$t+'*')){'already in PATH'}else{[Environment]::SetEnvironmentVariable('Path',($u.TrimEnd(';')+';'+$t),'User');'added to PATH'}"

echo  [3/3] Install plugin into DevEco Code auto-discover dir ...
if not exist "%PLUGIN%" mkdir "%PLUGIN%"
copy /Y "%SCRIPT_DIR%tsk.plugin.ts" "%PLUGIN%\tsk.ts" >nul

echo  [4/4] Register '/tsk' command (session command: show TSK report) ...
if not exist "%USERPROFILE%\.deveco\command" mkdir "%USERPROFILE%\.deveco\command"
if not exist "%USERPROFILE%\.config\opencode\command" mkdir "%USERPROFILE%\.config\opencode\command"
copy /Y "%SCRIPT_DIR%tsk.js" "%USERPROFILE%\.deveco\command\tsk.js" >nul
copy /Y "%SCRIPT_DIR%tsk.js" "%USERPROFILE%\.config\opencode\command\tsk.js" >nul
copy /Y "%SCRIPT_DIR%tsk.md" "%USERPROFILE%\.deveco\command\tsk.md" >nul
copy /Y "%SCRIPT_DIR%tsk.md" "%USERPROFILE%\.config\opencode\command\tsk.md" >nul

echo.
echo  Verify:
"%BIN%\tsk.exe" --version
if errorlevel 1 goto :fail_verify
echo  [OK] tsk binary works.
echo  [OK] plugin at: %PLUGIN%\tsk.ts
echo  [OK] '/tsk' command registered.
echo.
echo  DONE. Restart DevEco Code - plugin loads automatically.
echo  No deveco.json changes needed.
echo.
echo  NOTE: if 'tsk' command is still missing inside DevEco after restart,
echo        restart the computer once (PATH applies to all processes).
echo        The plugin itself does NOT need PATH - it falls back to
echo        %USERPROFILE%\.tsk\bin\tsk.exe automatically.
goto :end

:fail_core
echo  [FAIL] cannot copy tsk.exe (antivirus / permission?).
goto :end

:fail_verify
echo  [FAIL] tsk.exe failed to run (antivirus / permission?).
goto :end

:end
echo.
pause
