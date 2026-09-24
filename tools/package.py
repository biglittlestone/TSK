#!/usr/bin/env python3
"""Build the latest release binary and package the two distributable zips:
  - dist/tsk-v0.1.0.zip           (release binary package, install via bundled install.bat/sh)
  - dist/tsk-plugin-v0.1.0.zip    (Claude skills-dir plugin, install via its bundled installer)
Run from the repo root:  python tools/package.py
"""
import os
import shutil
import subprocess
import sys
import zipfile

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
DIST = os.path.join(ROOT, "dist")


def to_windows(s: str) -> str:
    return s.replace("\n", "\r\n")


# ---- release binary zip installer (uses bundled tsk.exe, does NOT rebuild) ----
RELEASE_SH = r'''#!/bin/sh
# TSK release installer - uses the bundled (latest) binary, does NOT rebuild.
set -e
BIN="$(cd "$(dirname "$0")" && pwd)/tsk"
mkdir -p "$HOME/bin"
cp "$BIN" "$HOME/bin/tsk"; chmod +x "$HOME/bin/tsk"
case ":$PATH:" in *":$HOME/bin:"*) ;; *) export PATH="$HOME/bin:$PATH"
   printf '\n# TSK\nexport PATH="$HOME/bin:$PATH"\n' >> "$HOME/.bashrc"
   [ -f "$HOME/.zshrc" ] && printf '\n# TSK\nexport PATH="$HOME/bin:$PATH"\n' >> "$HOME/.zshrc"
   echo "added ~/bin to PATH";; esac
echo "tsk -> $HOME/bin/tsk"
echo "next: cd your-project && tsk init"
'''

RELEASE_BAT = r'''@echo off
rem TSK release installer - uses the bundled tsk.exe, does NOT rebuild.
setlocal
set "BIN_DEST=%USERPROFILE%\bin"
mkdir "%BIN_DEST%" 2>nul
copy /Y "%~dp0tsk.exe" "%BIN_DEST%\tsk.exe" >nul
echo tsk.exe -^> %BIN_DEST%
powershell -NoProfile -Command "$p=[Environment]::GetEnvironmentVariable('Path','User'); $bin=$env:USERPROFILE+'\bin'; if($p -notmatch [regex]::Escape($bin)){[Environment]::SetEnvironmentVariable('Path',($bin+';'+$p),'User'); Write-Host 'added ~\bin to user PATH'} else {Write-Host '~\bin already on user PATH'}"
echo next: cd your-project ^&^& tsk init
echo.
pause
endlocal
'''

# ---- plugin zip installer (copy tsk + plugin, auto-enable, pause) ----
PLUGIN_SH = r'''#!/bin/sh
# TSK plugin release installer - ships inside the plugin zip. Uses the bundled
# (already-latest) tsk binary and copies the plugin. Does NOT rebuild.
set -e
D="$(cd "$(dirname "$0")" && pwd)"
mkdir -p "$HOME/bin"
if [ -f "$D/bin/tsk.exe" ]; then cp "$D/bin/tsk.exe" "$HOME/bin/tsk.exe"; chmod +x "$HOME/bin/tsk.exe" 2>/dev/null || true
elif [ -f "$D/bin/tsk" ]; then cp "$D/bin/tsk" "$HOME/bin/tsk"; chmod +x "$HOME/bin/tsk"; fi
case ":$PATH:" in *":$HOME/bin:"*) ;; *) export PATH="$HOME/bin:$PATH"
   printf '\n# TSK\nexport PATH="$HOME/bin:$PATH"\n' >> "$HOME/.bashrc"
   [ -f "$HOME/.zshrc" ] && printf '\n# TSK\nexport PATH="$HOME/bin:$PATH"\n' >> "$HOME/.zshrc"
   echo "added ~/bin to PATH";; esac
DEST="$HOME/.claude/skills/tsk"
mkdir -p "$(dirname "$DEST")"
for item in .claude-plugin SKILL.md commands bin; do cp -r "$D/$item" "$DEST/" 2>/dev/null || true; done
mkdir -p "$HOME/.tsk"
if [ ! -f "$HOME/.tsk/config.yaml" ]; then printf 'enabled: true
output_compression: off
' > "$HOME/.tsk/config.yaml"; echo "wrote default ~/.tsk/config.yaml"; fi
echo "tsk -> $HOME/bin  |  plugin -> $DEST"
echo "enabling plugin..."
claude plugin enable tsk@skills-dir >/dev/null 2>&1 || echo "run manually: claude plugin enable tsk@skills-dir"
echo "verify: tsk --version   &&   claude plugin list   (next session auto-loads as tsk@skills-dir)"
'''

PLUGIN_BAT = r'''@echo off
rem TSK plugin release installer - ships inside the plugin zip. Uses the bundled
rem (already-latest) tsk.exe and copies the plugin. Does NOT rebuild.
setlocal
set "BIN_DEST=%USERPROFILE%\bin"
mkdir "%BIN_DEST%" 2>nul
copy /Y "%~dp0bin\tsk.exe" "%BIN_DEST%\tsk.exe" >nul && echo tsk.exe -^> %BIN_DEST%
powershell -NoProfile -Command "$p=[Environment]::GetEnvironmentVariable('Path','User'); $bin=$env:USERPROFILE+'\bin'; if($p -notmatch [regex]::Escape($bin)){[Environment]::SetEnvironmentVariable('Path',($bin+';'+$p),'User'); Write-Host 'added ~\bin to user PATH'} else {Write-Host '~\bin already on user PATH'}"
set "DEST=%USERPROFILE%\.claude\skills\tsk"
mkdir "%DEST%" 2>nul
xcopy /E /I /Y "%~dp0.claude-plugin" "%DEST%\.claude-plugin" >nul
xcopy /E /I /Y "%~dp0commands" "%DEST%\commands" >nul
if exist "%~dp0SKILL.md" copy /Y "%~dp0SKILL.md" "%DEST%" >nul
if exist "%~dp0bin" xcopy /E /I /Y "%~dp0bin"
  mkdir "%USERPROFILE%\.tsk" 2>nul
  if not exist "%USERPROFILE%\.tsk\config.yaml" (echo enabled: true^&echo output_compression: off)>"%USERPROFILE%\.tsk\config.yaml" "%DEST%\bin" >nul 2>nul
echo plugin -^> %DEST%
claude plugin enable tsk@skills-dir >nul 2>&1 || echo run manually: claude plugin enable tsk@skills-dir
echo verify: tsk --version   ^&^&   claude plugin list   (next session auto-loads as tsk@skills-dir)
echo.
pause
endlocal
'''


# ---- uninstaller (shared, ships in both zips): plugin + binary, keeps stats ----
UNINSTALL_SH = r'''#!/bin/sh
# TSK uninstaller - removes the Claude plugin and the tsk binary.
# Keeps ~/.tsk (saved-token statistics); delete it manually if you want them gone.
set -e
rm -rf "$HOME/.claude/skills/tsk"
rm -f "$HOME/bin/tsk" "$HOME/bin/tsk.exe"
rm -rf "$HOME/.tsk"
echo "Removed: plugin, binary, stats (~/.tsk)"
'''

UNINSTALL_BAT = r'''@echo off
rem TSK uninstaller - removes the Claude plugin and the tsk binary.
rem Keeps %USERPROFILE%\.tsk (stats); delete it manually if you want them gone.
setlocal
rmdir /s /q "%USERPROFILE%\.claude\skills\tsk" 2>nul
del /q "%USERPROFILE%\bin\tsk.exe" 2>nul
del /q "%USERPROFILE%\bin\tsk" 2>nul
echo Removed: %USERPROFILE%\.claude\skills\tsk  and  tsk binary
rmdir /s /q "%USERPROFILE%\.tsk" 2>nul
echo Removed: plugin, binary, and stats (%USERPROFILE%\.tsk)
echo.
pause
endlocal
'''


def main() -> int:
    # 1) fresh release build (always, so the zips carry the latest code)
    print("[1/2] cargo build --release ...")
    r = subprocess.run(["cargo", "build", "--release"], cwd=ROOT, capture_output=True, text=True)
    if r.returncode != 0:
        print(r.stdout[-3000:]); print(r.stderr[-2000:])
        return 1
    release_exe = os.path.join(ROOT, "target", "release", "tsk.exe")
    print("       ", os.path.getsize(release_exe), "bytes")

    # 2) sync plugin source tree binary
    os.makedirs(os.path.join(DIST, "tsk-plugin", "bin"), exist_ok=True)
    shutil.copy2(release_exe, os.path.join(DIST, "tsk-plugin", "bin", "tsk.exe"))

    # release zip (top-level layout)
    tmp = os.path.join(DIST, "tmp-rel")
    shutil.rmtree(tmp, ignore_errors=True); os.makedirs(tmp)
    shutil.copy2(release_exe, os.path.join(tmp, "tsk.exe"))
    for f in ["README.md", "PROJECT.md", "LICENSE", "NOTICE"]:
        shutil.copy2(os.path.join(ROOT, f), os.path.join(tmp, f))
    open(os.path.join(tmp, "install.sh"), "w", encoding="utf-8", newline="\n").write(RELEASE_SH)
    open(os.path.join(tmp, "install.bat"), "wb").write(to_windows(RELEASE_BAT).encode("utf-8"))
    open(os.path.join(tmp, "uninstall.sh"), "w", encoding="utf-8", newline="\n").write(UNINSTALL_SH)
    open(os.path.join(tmp, "uninstall.bat"), "wb").write(to_windows(UNINSTALL_BAT).encode("utf-8"))
    os.makedirs(os.path.join(tmp, "examples", "sandbox"), exist_ok=True)
    for f in ["analyze_log.py", "git_stats.sh"]:
        shutil.copy2(os.path.join(ROOT, "examples", "sandbox", f), os.path.join(tmp, "examples", "sandbox", f))
    for src, dst in [
        (os.path.join(ROOT, "integration", "deveco", "tsk.plugin.ts"), os.path.join(tmp, "integration", "deveco", "tsk.plugin.ts")),
        (os.path.join(ROOT, "integration", "deveco", "README.md"),    os.path.join(tmp, "integration", "deveco", "README.md")),
        (os.path.join(ROOT, "integration", "deveco", "DEVECO_USAGE.md"), os.path.join(tmp, "integration", "deveco", "DEVECO_USAGE.md")),
        (os.path.join(ROOT, "integration", "deveco", "smoke.test.mjs"), os.path.join(tmp, "integration", "deveco", "smoke.test.mjs")),
    ]:
        os.makedirs(os.path.dirname(dst), exist_ok=True)
        shutil.copy2(src, dst)
    with zipfile.ZipFile(os.path.join(DIST, "tsk-v0.1.0.zip"), "w", zipfile.ZIP_DEFLATED) as z:
        for base, _, fs in os.walk(tmp):
            for f in fs:
                fp = os.path.join(base, f)
                z.write(fp, os.path.relpath(fp, tmp))
    shutil.rmtree(tmp, ignore_errors=True)

    # plugin zip (tsk-plugin/ root, installers injected)
    with zipfile.ZipFile(os.path.join(DIST, "tsk-plugin-v0.1.0.zip"), "w", zipfile.ZIP_DEFLATED) as z:
        for base, _, fs in os.walk(os.path.join(DIST, "tsk-plugin")):
            for f in fs:
                fp = os.path.join(base, f)
                z.write(fp, os.path.join("tsk-plugin", os.path.relpath(fp, os.path.join(DIST, "tsk-plugin"))))
        z.writestr("tsk-plugin/install.sh", PLUGIN_SH)
        z.writestr("tsk-plugin/install.bat", to_windows(PLUGIN_BAT))
        z.writestr("tsk-plugin/uninstall.sh", UNINSTALL_SH)
        z.writestr("tsk-plugin/uninstall.bat", to_windows(UNINSTALL_BAT))

    print("[2/2] packaged:")
    for name in sorted(os.listdir(DIST)):
        p = os.path.join(DIST, name)
        if os.path.isfile(p) and name.endswith(".zip"):
            print(f"        {name:26} {os.path.getsize(p):,} B")
    return 0


if __name__ == "__main__":
    sys.exit(main())