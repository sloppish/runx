#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
APP_NAME="Runx"
APP_DIR="/Applications"
ENABLE_LOGIN_ITEM=0
DRY_RUN=0
SIGN_IDENTITY=""

usage() {
  cat <<'EOF'
Usage: scripts/install-macos.sh [options]

Options:
  --app-dir PATH         Install destination directory (default: /Applications)
  --user-apps            Shortcut for --app-dir "$HOME/Applications"
  --login-item           Add Runx to macOS Login Items after install
  --sign-identity NAME   Code-signing identity to pass through to packaging
  --dry-run              Print the actions without installing
  -h, --help             Show this help
EOF
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --app-dir)
      APP_DIR="$2"
      shift 2
      ;;
    --user-apps)
      APP_DIR="$HOME/Applications"
      shift
      ;;
    --login-item)
      ENABLE_LOGIN_ITEM=1
      shift
      ;;
    --sign-identity)
      SIGN_IDENTITY="$2"
      shift 2
      ;;
    --dry-run)
      DRY_RUN=1
      shift
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      echo "Unknown argument: $1" >&2
      usage >&2
      exit 1
      ;;
  esac
done

if [[ "$DRY_RUN" -eq 1 ]]; then
  echo "Would package Runx.app into $APP_DIR"
else
  PACKAGE_ARGS=()
  if [[ -n "$SIGN_IDENTITY" ]]; then
    PACKAGE_ARGS+=(--sign-identity "$SIGN_IDENTITY")
  fi
  "$ROOT_DIR/scripts/package-macos.sh" "${PACKAGE_ARGS[@]}"
fi

SOURCE_APP="$ROOT_DIR/dist/${APP_NAME}.app"
TARGET_APP="$APP_DIR/${APP_NAME}.app"

copy_app() {
  mkdir -p "$APP_DIR"
  rm -rf "$TARGET_APP"
  ditto "$SOURCE_APP" "$TARGET_APP"
}

copy_app_with_admin() {
  local shell_command
  shell_command=$(printf 'mkdir -p %q && rm -rf %q && ditto %q %q' "$APP_DIR" "$TARGET_APP" "$SOURCE_APP" "$TARGET_APP")

  osascript <<'APPLESCRIPT' "$shell_command"
on run argv
  do shell script item 1 of argv with administrator privileges
end run
APPLESCRIPT
}

if [[ "$DRY_RUN" -eq 1 ]]; then
  printf 'Would install %s to %s\n' "$SOURCE_APP" "$TARGET_APP"
else
  if [[ -w "$APP_DIR" || (! -e "$APP_DIR" && -w "$(dirname "$APP_DIR")") ]]; then
    copy_app
  else
    copy_app_with_admin
  fi
  printf 'Installed %s\n' "$TARGET_APP"
fi

if [[ "$ENABLE_LOGIN_ITEM" -eq 1 ]]; then
  if [[ "$DRY_RUN" -eq 1 ]]; then
    "$ROOT_DIR/scripts/login-item-macos.sh" --dry-run enable "$TARGET_APP" "$APP_NAME"
  else
    "$ROOT_DIR/scripts/login-item-macos.sh" enable "$TARGET_APP" "$APP_NAME"
  fi
fi
