#!/usr/bin/env bash
set -euo pipefail
ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"
bash script/build.sh
STAGE_DIR="$(mktemp -d)"
trap 'rm -rf "$STAGE_DIR"' EXIT
ditto dist/SmartClash.app "$STAGE_DIR/SmartClash.app"
ln -s /Applications "$STAGE_DIR/Applications"
cp README.md "$STAGE_DIR/使用说明.md"
hdiutil create -volname SmartClash -srcfolder "$STAGE_DIR" -ov -format UDZO dist/SmartClash-Intel.dmg
if [[ -n "${NOTARY_PROFILE:-}" ]]; then
  xcrun notarytool submit dist/SmartClash-Intel.dmg --keychain-profile "$NOTARY_PROFILE" --wait
  xcrun stapler staple dist/SmartClash-Intel.dmg
fi
