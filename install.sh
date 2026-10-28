#!/usr/bin/env bash
# Install wordesign for the current user (no sudo needed):
#   ./install.sh            -> ~/.local  (app menu entry + `wordesign` command)
#   PREFIX=/usr/local sudo -E ./install.sh   -> system-wide
set -euo pipefail
cd "$(dirname "$0")"

PREFIX="${PREFIX:-$HOME/.local}"
BIN="$PREFIX/bin"
APPS="$PREFIX/share/applications"
ICONS="$PREFIX/share/icons/hicolor"

# 1. Get a binary: the bundled one if it runs here, otherwise build from source.
exe=""
if [ -x bin/wordesign ] && ! ldd bin/wordesign 2>&1 | grep -q "not found"; then
  exe=bin/wordesign
else
  if ! command -v cargo >/dev/null; then
    echo "No bundled binary works here and Rust is missing."
    echo "Install Rust 1.95+ from https://rustup.rs and run this script again."
    exit 1
  fi
  echo "Building wordesign (a few minutes the first time)..."
  cargo build --release
  exe=target/release/wordesign
fi

# 2. Is an older (or the same) version already installed? Say so, and update it in place.
new_ver="$("$exe" --version 2>/dev/null | awk '{print $2}')"
old_ver=""; had=0
if [ -x "$BIN/wordesign" ]; then
  had=1
  # Read the stamp left by the previous install (never run the old binary: v1.1 would open a window).
  old_ver="$(cat "$PREFIX/share/wordesign/VERSION" 2>/dev/null || true)"
  echo "Found wordesign ${old_ver:-(an older version)} in $BIN; updating to ${new_ver:-this version}."
elif command -v wordesign >/dev/null && [ "$(command -v wordesign)" != "$BIN/wordesign" ]; then
  echo "Note: another wordesign exists at $(command -v wordesign); it is left alone."
fi
if pgrep -x wordesign >/dev/null 2>&1; then
  echo "wordesign is running right now: the update applies the next time you start it."
fi

# 3. Copy everything into place. The binary goes via a temp file + rename, which is safe even
#    while the old one is running (no "text file busy").
mkdir -p "$BIN"
install -m755 "$exe" "$BIN/.wordesign.new"
mv -f "$BIN/.wordesign.new" "$BIN/wordesign"
mkdir -p "$PREFIX/share/wordesign"
printf '%s\n' "$new_ver" > "$PREFIX/share/wordesign/VERSION"
for s in 48 64 128 256 512; do
  install -Dm644 "assets/wordesign-$s.png" "$ICONS/${s}x${s}/apps/wordesign.png"
done
install -Dm644 packaging/wordesign.desktop "$APPS/wordesign.desktop"
# Make the menu entry point at the exact binary, so it works even if ~/.local/bin is not on PATH.
sed -i "s|^Exec=.*|Exec=$BIN/wordesign|" "$APPS/wordesign.desktop"

command -v update-desktop-database >/dev/null && update-desktop-database "$APPS" 2>/dev/null || true
command -v gtk-update-icon-cache >/dev/null && gtk-update-icon-cache -qf "$ICONS" 2>/dev/null || true

if [ -n "$old_ver" ] && [ "$old_ver" = "$new_ver" ]; then echo "Reinstalled $new_ver."; elif [ "$had" = 1 ]; then echo "Updated ${old_ver:-the older version} -> $new_ver."; else echo "Installed $new_ver."; fi
echo "Find 'wordesign' in your app menu, or run: $BIN/wordesign"
case ":$PATH:" in *":$BIN:"*) ;; *) echo "(Add $BIN to your PATH to type just 'wordesign'.)" ;; esac
echo "Your prompts live in ~/.wordesign and are left alone by install and uninstall."
