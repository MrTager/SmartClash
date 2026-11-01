#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")"
build_status=0
if bash script/package.sh; then
  open dist
  echo '构建完成：dist/SmartClash-Intel.dmg。'
else
  build_status=1
  echo '构建失败，请保留上面的错误信息。需要 Xcode 15+ 或 Swift 5.9+ Command Line Tools，以及可访问 GitHub 的网络。'
fi
read -r -p '按回车关闭窗口。'

exit "$build_status"
