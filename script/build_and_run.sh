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
  *) echo 'usage: build_and_run.sh [run|--debug|--logs|--telemetry|--verify]' >&2; exit 2 ;;
esac
