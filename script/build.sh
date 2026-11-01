#!/usr/bin/env bash
set -euo pipefail
ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"
[[ "$(uname -s)" == Darwin ]] || { echo '请在 macOS 上运行，需要 Xcode / Command Line Tools。' >&2; exit 1; }
xcrun --find swift >/dev/null
if [[ ! -x Resources/mihomo ]]; then bash script/fetch_core.sh; fi
swift test --arch x86_64
swift build --arch x86_64 -c release
BIN_DIR="$(swift build --arch x86_64 -c release --show-bin-path)"
APP_BUNDLE="$ROOT_DIR/dist/SmartClash.app"
# A running app must restore network settings through its regular quit handler.
if pgrep -x SmartClash >/dev/null; then
  osascript -e 'tell application id "com.smartclash.app" to quit'
  for ((i=0; i<60; i++)); do
    if ! pgrep -x SmartClash >/dev/null; then break; fi
    sleep 1
  done
  if pgrep -x SmartClash >/dev/null; then echo '请先在客户端停止代理并正常退出。' >&2; exit 1; fi
fi
rm -rf "$APP_BUNDLE"
mkdir -p "$APP_BUNDLE/Contents/MacOS" "$APP_BUNDLE/Contents/Resources"
cp "$BIN_DIR/SmartClash" "$APP_BUNDLE/Contents/MacOS/SmartClash"
cp Resources/mihomo "$APP_BUNDLE/Contents/Resources/mihomo"
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
<key>CFBundleShortVersionString</key><string>0.4.0</string>
<key>CFBundleVersion</key><string>4</string>
<key>LSMinimumSystemVersion</key><string>13.0</string>
<key>NSPrincipalClass</key><string>NSApplication</string>
<key>NSHighResolutionCapable</key><true/>
</dict></plist>
PLIST
plutil -lint "$APP_BUNDLE/Contents/Info.plist"
SIGNING_IDENTITY="${SIGNING_IDENTITY:--}"
if [[ "$SIGNING_IDENTITY" == '-' ]]; then
  codesign --force --sign - "$APP_BUNDLE/Contents/Resources/mihomo"
  codesign --force --sign - "$APP_BUNDLE"
else
  codesign --force --options runtime --timestamp --sign "$SIGNING_IDENTITY" "$APP_BUNDLE/Contents/Resources/mihomo"
  codesign --force --options runtime --timestamp --sign "$SIGNING_IDENTITY" "$APP_BUNDLE"
fi
codesign --verify --deep --strict "$APP_BUNDLE"
lipo -verify_arch x86_64 "$APP_BUNDLE/Contents/MacOS/SmartClash"
echo "已构建：$APP_BUNDLE"
