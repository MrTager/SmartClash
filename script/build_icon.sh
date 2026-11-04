#!/usr/bin/env bash
set -euo pipefail
ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"
[[ -f Resources/AppIcon.png ]] || { echo '缺少应用图标 Resources/AppIcon.png' >&2; exit 1; }
if [[ -f Resources/AppIcon.icns && Resources/AppIcon.icns -nt Resources/AppIcon.png && Resources/AppIcon.icns -nt script/build_icon.sh ]]; then exit 0; fi
ICON_WORK_DIR="$(mktemp -d)"
trap 'rm -rf "$ICON_WORK_DIR"' EXIT
ICONSET="$ICON_WORK_DIR/AppIcon.iconset"
mkdir "$ICONSET"
for size in 16 32 128 256 512; do
  sips -z "$size" "$size" Resources/AppIcon.png --out "$ICONSET/icon_${size}x${size}.png" >/dev/null
  retina=$((size * 2))
  sips -z "$retina" "$retina" Resources/AppIcon.png --out "$ICONSET/icon_${size}x${size}@2x.png" >/dev/null
done
iconutil -c icns "$ICONSET" -o Resources/AppIcon.icns
