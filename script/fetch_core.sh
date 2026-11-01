#!/usr/bin/env bash
set -euo pipefail
ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
CORE_VERSION="v1.19.33"
CORE_ASSET="mihomo-darwin-amd64-compatible-${CORE_VERSION}.gz"
mkdir -p "$ROOT_DIR/Resources"
if [[ -n "${MIHOMO_BINARY:-}" ]]; then
  cp "$MIHOMO_BINARY" "$ROOT_DIR/Resources/mihomo"
  chmod 755 "$ROOT_DIR/Resources/mihomo"
  /usr/bin/file "$ROOT_DIR/Resources/mihomo" | /usr/bin/grep -q 'x86_64'
  "$ROOT_DIR/Resources/mihomo" -v
  exit 0
fi
TEMP_DIR="$(mktemp -d)"
trap 'rm -rf "$TEMP_DIR"' EXIT
/usr/bin/curl --fail --location --retry 3 --connect-timeout 15 --max-time 90 \
  --proto '=https' --proto-redir '=https' \
  "https://api.github.com/repos/MetaCubeX/mihomo/releases/tags/$CORE_VERSION" -o "$TEMP_DIR/release.json"
/usr/bin/xcrun swift "$ROOT_DIR/script/Asset.swift" "$TEMP_DIR/release.json" "$CORE_ASSET" > "$TEMP_DIR/asset.txt"
CORE_URL="$(sed -n '1p' "$TEMP_DIR/asset.txt")"
CORE_SHA="$(sed -n '2p' "$TEMP_DIR/asset.txt")"
/usr/bin/curl --fail --location --retry 3 --connect-timeout 15 --max-time 600 \
  --proto '=https' --proto-redir '=https' "$CORE_URL" -o "$TEMP_DIR/core.gz"
printf '%s  %s\n' "$CORE_SHA" "$TEMP_DIR/core.gz" | /usr/bin/shasum -a 256 -c -
/usr/bin/gzip -dc "$TEMP_DIR/core.gz" > "$ROOT_DIR/Resources/mihomo"
chmod 755 "$ROOT_DIR/Resources/mihomo"
/usr/bin/file "$ROOT_DIR/Resources/mihomo" | /usr/bin/grep -q 'x86_64'
"$ROOT_DIR/Resources/mihomo" -v
cp "$TEMP_DIR/release.json" "$ROOT_DIR/Resources/core-release.json"
