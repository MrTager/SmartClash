#!/usr/bin/env bash
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
python3 "$ROOT/Scripts/generate_project.py"
plutil -lint "$ROOT/SmartClash.xcodeproj/project.pbxproj" "$ROOT/Config/App.Info.plist" "$ROOT/Config/PacketTunnel.Info.plist" "$ROOT/Config/VPN.entitlements"
xcodebuild -project "$ROOT/SmartClash.xcodeproj" -scheme SmartClash-Manager -sdk iphonesimulator -destination 'generic/platform=iOS Simulator' -derivedDataPath "$ROOT/DerivedData" CODE_SIGNING_ALLOWED=NO build
xcodebuild -project "$ROOT/SmartClash.xcodeproj" -scheme SmartClash-VPN -sdk iphoneos -destination 'generic/platform=iOS' -derivedDataPath "$ROOT/DerivedDataVPN" CODE_SIGNING_ALLOWED=NO build
if [[ -n "${IOS_TEST_DESTINATION:-}" ]]; then
  xcodebuild -project "$ROOT/SmartClash.xcodeproj" -scheme SmartClash-Manager -destination "$IOS_TEST_DESTINATION" -derivedDataPath "$ROOT/DerivedData" CODE_SIGNING_ALLOWED=NO test
fi
