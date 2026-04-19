#!/usr/bin/env bash
set -euo pipefail

ACTION="${1:-}"
APP_PATH="${2:-/Applications/Runx.app}"
APP_NAME="${3:-Runx}"
DRY_RUN=0

if [[ "$ACTION" == "--dry-run" ]]; then
  ACTION="${2:-}"
  APP_PATH="${3:-/Applications/Runx.app}"
  APP_NAME="${4:-Runx}"
  DRY_RUN=1
fi

usage() {
  cat <<'EOF'
Usage:
  scripts/login-item-macos.sh enable [APP_PATH] [APP_NAME]
  scripts/login-item-macos.sh disable [APP_PATH] [APP_NAME]
  scripts/login-item-macos.sh status [APP_PATH] [APP_NAME]
  scripts/login-item-macos.sh --dry-run enable [APP_PATH] [APP_NAME]
EOF
}

if [[ -z "$ACTION" ]]; then
  usage >&2
  exit 1
fi

run_applescript() {
  if [[ "$DRY_RUN" -eq 1 ]]; then
    printf 'osascript %s %s %s\n' "$ACTION" "$APP_PATH" "$APP_NAME"
    return 0
  fi

  osascript <<'APPLESCRIPT' "$ACTION" "$APP_PATH" "$APP_NAME"
on run argv
  set actionName to item 1 of argv
  set appPath to item 2 of argv
  set appName to item 3 of argv

  tell application "System Events"
    if actionName is "enable" then
      repeat with existingItem in login items
        if name of existingItem is appName then
          delete existingItem
          exit repeat
        end if
      end repeat
      make login item at end with properties {name:appName, path:appPath, hidden:false}
      return "enabled"
    else if actionName is "disable" then
      repeat with existingItem in login items
        if name of existingItem is appName then
          delete existingItem
          return "disabled"
        end if
      end repeat
      return "missing"
    else if actionName is "status" then
      repeat with existingItem in login items
        if name of existingItem is appName then
          return "enabled"
        end if
      end repeat
      return "disabled"
    else
      error "unknown action"
    end if
  end tell
end run
APPLESCRIPT
}

case "$ACTION" in
  enable|disable|status)
    run_applescript
    ;;
  *)
    usage >&2
    exit 1
    ;;
esac
