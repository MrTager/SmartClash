#!/usr/bin/env bash
set -euo pipefail
ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"
bash script/build.sh
case "${1:-run}" in
  run) open -n dist/SmartClash.app ;;
  --debug) lldb -- dist/SmartClash.app/Contents/MacOS/SmartClash ;;
  --logs|--telemetry) open -n dist/SmartClash.app; /usr/bin/log stream --info --style compact --predicate 'process == "SmartClash"' ;;
  --verify) open -n dist/SmartClash.app; sleep 2; pgrep -x SmartClash >/dev/null ;;
  --install)
    if [[ -e /Applications/SmartClash.app ]]; then
      echo '/Applications/SmartClash.app 已存在；请先备份旧应用，避免直接覆盖。' >&2
      exit 1
    fi
    ditto dist/SmartClash.app /Applications/SmartClash.app
    codesign --verify --deep --strict /Applications/SmartClash.app
    open -n /Applications/SmartClash.app
    sleep 2
    pgrep -x SmartClash >/dev/null
    ;;
  --update)
    if pgrep -x SmartClash >/dev/null; then
      echo '当前应用仍在运行。请先允许正常停止代理并退出，再安装；不会擅自中断连接。' >&2; exit 1
    fi
    if [[ ! -d /Applications/SmartClash.app ]] ||
       [[ "$(/usr/libexec/PlistBuddy -c 'Print CFBundleIdentifier' /Applications/SmartClash.app/Contents/Info.plist)" != com.smartclash.app ]]; then
      echo '未发现可安全更新的 SmartClash.app。' >&2; exit 1
    fi
    STAGE_DIR="$(mktemp -d /Applications/.SmartClash-update.XXXXXX)"
    BACKUP_APP="$ROOT_DIR/dist/SmartClash-installed-backup-$(date +%Y%m%d-%H%M%S).app"
    [[ ! -e "$BACKUP_APP" ]] || { rmdir "$STAGE_DIR"; exit 1; }
    ditto dist/SmartClash.app "$STAGE_DIR/SmartClash.app"
    codesign --verify --deep --strict "$STAGE_DIR/SmartClash.app"
    mv /Applications/SmartClash.app "$BACKUP_APP"
    if ! mv "$STAGE_DIR/SmartClash.app" /Applications/SmartClash.app; then
      mv "$BACKUP_APP" /Applications/SmartClash.app
      echo '安装失败，已恢复旧应用。' >&2; exit 1
    fi
    rmdir "$STAGE_DIR"
    echo "旧应用已备份：$BACKUP_APP"
    open -n /Applications/SmartClash.app
    sleep 2
    pgrep -x SmartClash >/dev/null
    ;;
  *) echo 'usage: build_and_run.sh [run|--debug|--logs|--telemetry|--verify|--install|--update]' >&2; exit 2 ;;
esac
