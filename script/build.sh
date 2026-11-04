#!/usr/bin/env bash
set -euo pipefail
ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"
[[ "$(uname -s)" == Darwin ]] || { echo '请在 macOS 上运行，需要 Xcode / Command Line Tools。' >&2; exit 1; }
xcrun --find swift >/dev/null
bash script/build_icon.sh
if [[ ! -x Resources/mihomo ]]; then bash script/fetch_core.sh; fi
swift test --arch x86_64
swift build --arch x86_64 -c release
BIN_DIR="$(swift build --arch x86_64 -c release --show-bin-path)"
APP_BUNDLE="$ROOT_DIR/dist/SmartClash.app"
# Never interrupt the installed client just to build a separate dist bundle.
if pgrep -x SmartClash >/dev/null && /usr/sbin/lsof -a -c SmartClash -d txt -Fn 2>/dev/null | /usr/bin/grep -Fx "n$APP_BUNDLE/Contents/MacOS/SmartClash" >/dev/null; then
  echo 'dist 中的应用正在运行，请先正常退出该副本再构建；不会自动中断网络。' >&2; exit 1
fi
rm -rf "$APP_BUNDLE"
mkdir -p "$APP_BUNDLE/Contents/MacOS" "$APP_BUNDLE/Contents/Resources" "$APP_BUNDLE/Contents/Library/HelperTools"
cp "$BIN_DIR/SmartClash" "$APP_BUNDLE/Contents/MacOS/SmartClash"
cp "$BIN_DIR/SmartClashHelper" "$APP_BUNDLE/Contents/Library/HelperTools/SmartClashHelper"
for RESOURCE_BUNDLE in "$BIN_DIR"/SmartClash_*.bundle; do
  [[ -d "$RESOURCE_BUNDLE" ]] || continue
  ditto "$RESOURCE_BUNDLE" "$APP_BUNDLE/Contents/Resources/$(basename "$RESOURCE_BUNDLE")"
done
cp Resources/mihomo "$APP_BUNDLE/Contents/Resources/mihomo"
cp Resources/AppIcon.icns "$APP_BUNDLE/Contents/Resources/AppIcon.icns"
cp LICENSE THIRD_PARTY_NOTICES.md "$APP_BUNDLE/Contents/Resources/"
YAMS_SOURCE="$ROOT_DIR/.build/checkouts/Yams"
if [[ ! -d "$YAMS_SOURCE" ]]; then YAMS_SOURCE="$ROOT_DIR/.build/checkouts/yams"; fi
if [[ ! -d "$YAMS_SOURCE" ]]; then
  echo 'Missing resolved Yams source and license notices.' >&2; exit 1
fi
mkdir -p "$APP_BUNDLE/Contents/Resources/Notices/Yams"
while IFS= read -r notice; do
  relative="${notice#"$YAMS_SOURCE/"}"
  destination="$APP_BUNDLE/Contents/Resources/Notices/Yams/$relative"
  mkdir -p "$(dirname "$destination")"
  cp "$notice" "$destination"
done < <(find "$YAMS_SOURCE" -type f \( -iname '*license*' -o -iname '*copying*' -o -iname '*notice*' \))
cat > "$APP_BUNDLE/Contents/Info.plist" <<'PLIST'
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
<key>CFBundleExecutable</key><string>SmartClash</string>
<key>CFBundleIdentifier</key><string>com.smartclash.app</string>
<key>CFBundleName</key><string>SmartClash</string>
<key>CFBundleDisplayName</key><string>SmartClash</string>
<key>CFBundlePackageType</key><string>APPL</string>
<key>CFBundleIconFile</key><string>AppIcon</string>
<key>CFBundleShortVersionString</key><string>0.7.1</string>
<key>CFBundleVersion</key><string>11</string>
<key>LSMinimumSystemVersion</key><string>13.0</string>
<key>NSPrincipalClass</key><string>NSApplication</string>
<key>NSHighResolutionCapable</key><true/>
</dict></plist>
PLIST
plutil -lint "$APP_BUNDLE/Contents/Info.plist"
SIGNING_IDENTITY="${SIGNING_IDENTITY:--}"
if [[ "$SIGNING_IDENTITY" == '-' ]]; then
  codesign --force --sign - "$APP_BUNDLE/Contents/Resources/mihomo"
  codesign --force --options runtime --sign - "$APP_BUNDLE/Contents/Library/HelperTools/SmartClashHelper"
  codesign --force --options runtime --sign - "$APP_BUNDLE"
else
  codesign --force --options runtime --timestamp --sign "$SIGNING_IDENTITY" "$APP_BUNDLE/Contents/Resources/mihomo"
  codesign --force --options runtime --timestamp --sign "$SIGNING_IDENTITY" "$APP_BUNDLE/Contents/Library/HelperTools/SmartClashHelper"
  codesign --force --options runtime --timestamp --sign "$SIGNING_IDENTITY" "$APP_BUNDLE"
fi
codesign --verify --deep --strict "$APP_BUNDLE"
lipo "$APP_BUNDLE/Contents/MacOS/SmartClash" -verify_arch x86_64
lipo "$APP_BUNDLE/Contents/Library/HelperTools/SmartClashHelper" -verify_arch x86_64
echo "已构建：$APP_BUNDLE"
