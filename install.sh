#!/bin/sh
# TSK install script (source-tree entry). ALWAYS rebuilds from the latest source — it never
# falls back to a possibly-stale prebuilt artifact.
#
# Usage:
#   ./install.sh           rebuild + install `tsk` to ~/bin (auto-adds ~/bin to PATH)
#   ./install.sh --plugin  same, and also install as a Claude skills-dir plugin
#
# NOTE: the *release zip* carries its own install.sh which uses the bundled
# (already-latest) binary instead of rebuilding — that one lives inside the zip.
set -e
D="$(cd "$(dirname "$0")" && pwd)"
PLUGIN=0
[ "$1" = "--plugin" ] && PLUGIN=1

echo "[1/1] building release (always fresh)…"
(cd "$D" && cargo build --release >/dev/null)

mkdir -p "$HOME/bin"
cp "$D/target/release/tsk" "$HOME/bin/tsk"
chmod +x "$HOME/bin/tsk"
echo "      tsk -> $HOME/bin/tsk"

case ":$PATH:" in
  *":$HOME/bin:"*) ;;
  *) export PATH="$HOME/bin:$PATH"
     printf '\n# TSK\nexport PATH="$HOME/bin:$PATH"\n' >> "$HOME/.bashrc"
     [ -f "$HOME/.zshrc" ] && printf '\n# TSK\nexport PATH="$HOME/bin:$PATH"\n' >> "$HOME/.zshrc"
     echo "      added ~/bin to PATH (.bashrc / .zshrc; open a new shell or source it)"
     ;;
esac

if [ "$PLUGIN" = 1 ]; then
  DEST="$HOME/.claude/skills/tsk"
  mkdir -p "$(dirname "$DEST")"
  cp -r "$D/dist/tsk-plugin/." "$DEST"  mkdir -p "$HOME/.tsk"
  if [ ! -f "$HOME/.tsk/config.yaml" ]; then printf 'enabled: true
output_compression: off
' > "$HOME/.tsk/config.yaml"; echo "wrote default ~/.tsk/config.yaml"; fi

  echo "[2/2] plugin -> $DEST   (auto-loads next session as tsk@skills-dir)"
  echo "enabling plugin…"
  claude plugin enable tsk@skills-dir >/dev/null 2>&1 || echo "run manually: claude plugin enable tsk@skills-dir"
  echo
  printf 'verify: tsk --version && claude plugin list\n'
  exit 0
fi

echo
echo 'next: cd your-project && tsk init'