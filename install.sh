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

# 2. Copy everything into place.
install -Dm755 "$exe" "$BIN/wordesign"
for s in 48 64 128 256 512; do
  install -Dm644 "assets/wordesign-$s.png" "$ICONS/${s}x${s}/apps/wordesign.png"
done
install -Dm644 packaging/wordesign.desktop "$APPS/wordesign.desktop"
# Make the menu entry point at the exact binary, so it works even if ~/.local/bin is not on PATH.
sed -i "s|^Exec=.*|Exec=$BIN/wordesign|" "$APPS/wordesign.desktop"

command -v update-desktop-database >/dev/null && update-desktop-database "$APPS" 2>/dev/null || true
command -v gtk-update-icon-cache >/dev/null && gtk-update-icon-cache -qf "$ICONS" 2>/dev/null || true

echo "Installed. Find 'wordesign' in your app menu, or run: $BIN/wordesign"
case ":$PATH:" in *":$BIN:"*) ;; *) echo "(Add $BIN to your PATH to type just 'wordesign'.)" ;; esac
echo "Your prompts live in ~/.wordesign and are left alone by install and uninstall."
