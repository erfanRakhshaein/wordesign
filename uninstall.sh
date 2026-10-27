#!/usr/bin/env bash
# Remove wordesign (same PREFIX rules as install.sh). Your prompts in ~/.wordesign are NOT touched.
set -euo pipefail
PREFIX="${PREFIX:-$HOME/.local}"
rm -f "$PREFIX/bin/wordesign" "$PREFIX/share/applications/wordesign.desktop"
for s in 48 64 128 256 512; do rm -f "$PREFIX/share/icons/hicolor/${s}x${s}/apps/wordesign.png"; done
command -v update-desktop-database >/dev/null && update-desktop-database "$PREFIX/share/applications" 2>/dev/null || true
echo "wordesign removed. Your prompts in ~/.wordesign were kept."
