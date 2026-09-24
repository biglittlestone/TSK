@echo off
rem TSK install script (source-tree entry). ALWAYS rebuilds the latest binary -
rem never reuses a possibly-stale prebuilt artifact.
rem
rem Usage:
rem   install.bat              rebuild + install tsk.exe to ~\bin (auto-add to user PATH)
rem   install.bat --plugin     same, and also install as a Claude skills-dir plugin
rem
rem NOTE: the release zip carries its own install.bat that uses the bundled
rem (already-latest) tsk.exe without rebuilding - that one lives inside the zip.
setlocal
set "PLUGIN=0"
if /i "%1"=="--plugin" set "PLUGIN=1"

where cargo >nul 2>nul
if errorlevel 1 (
  echo [tsk] cargo not found. Use the release zip's install.bat instead.
  goto :end
)

echo [1/1] building release (always fresh^)...
pushd "%~dp0"
cargo build --release >nul
popd

set "BIN_DEST=%USERPROFILE%\bin"
mkdir "%BIN_DEST%" 2>nul
copy /Y "%~dp0target\release\tsk.exe" "%BIN_DEST%\tsk.exe" >nul
echo       tsk.exe -^> %BIN_DEST%
powershell -NoProfile -Command "$p=[Environment]::GetEnvironmentVariable('Path','User'); $bin=$env:USERPROFILE+'\bin'; if($p -notmatch [regex]::Escape($bin)){[Environment]::SetEnvironmentVariable('Path',($bin+';'+$p),'User'); Write-Host '      added ~\bin to user PATH (new terminals see it)'} else {Write-Host '      ~\bin already on user PATH'}"

if "%PLUGIN%"=="1" (
  set "DEST=%USERPROFILE%\.claude\skills\tsk"
  mkdir "%DEST%" 2>nul
  xcopy /E /I /Y "%~dp0dist\tsk-plugin\." "%DEST%" >nul
  echo [2/2] plugin -^> %DEST%   (auto-loads next session as tsk@skills-dir^)
  claude plugin enable tsk@skills-dir >nul 2>&1 || echo run manually: claude plugin enable tsk@skills-dir
  echo.
  echo verify: tsk --version ^&^& claude plugin list
) else (
  echo.
  echo next: cd your-project ^&^& tsk init
)
:end
echo.
pause
endlocal