#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
APP_NAME="Runx"
APP_BUNDLE_ID="dev.runx.launcher"
PROFILE="release"
OUT_DIR="$ROOT_DIR/dist"
PASS_RANK_BIN="${RUNX_PASS_RANK_BIN:-}"

usage() {
  cat <<'EOF'
Usage: scripts/package-macos.sh [options]

Options:
  --out-dir PATH         Bundle output directory (default: ./dist)
  --pass-rank-bin PATH   Compiled pass_rank binary to bundle
  --debug                Build with the debug profile instead of release
  -h, --help             Show this help
EOF
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --out-dir)
      OUT_DIR="$2"
      shift 2
      ;;
    --pass-rank-bin)
      PASS_RANK_BIN="$2"
      shift 2
      ;;
    --debug)
      PROFILE="debug"
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

resolve_pass_rank_bin() {
  local candidates=()
  if [[ -n "${PASS_RANK_BIN}" ]]; then
    candidates+=("${PASS_RANK_BIN}")
  fi

  candidates+=(
    "$HOME/Library/Application Support/runx/plugins/pass/pass_rank"
    "$ROOT_DIR/../alfred_pass/pass_rank/pass_rank"
  )

  local candidate
  for candidate in "${candidates[@]}"; do
    if [[ -x "${candidate}" ]]; then
      printf '%s\n' "${candidate}"
      return 0
    fi
  done

  echo "Could not find a compiled pass_rank binary. Provide --pass-rank-bin PATH." >&2
  exit 1
}

PASS_RANK_BIN="$(resolve_pass_rank_bin)"
APP_VERSION="$(awk -F '"' '/^version = / { print $2; exit }' "$ROOT_DIR/Cargo.toml")"

if [[ "${PROFILE}" == "release" ]]; then
  cargo build --release --manifest-path "$ROOT_DIR/Cargo.toml"
  BIN_PATH="$ROOT_DIR/target/release/runx"
else
  cargo build --manifest-path "$ROOT_DIR/Cargo.toml"
  BIN_PATH="$ROOT_DIR/target/debug/runx"
fi

BUNDLE_PATH="$OUT_DIR/${APP_NAME}.app"
CONTENTS_PATH="$BUNDLE_PATH/Contents"
MACOS_PATH="$CONTENTS_PATH/MacOS"
RESOURCES_PATH="$CONTENTS_PATH/Resources"
DEFAULTS_PATH="$RESOURCES_PATH/defaults"

rm -rf "$BUNDLE_PATH"
mkdir -p "$MACOS_PATH" "$DEFAULTS_PATH/pass"

cp "$BIN_PATH" "$MACOS_PATH/runx"
chmod 755 "$MACOS_PATH/runx"
cp "$ROOT_DIR/plugins/pass.lua" "$DEFAULTS_PATH/pass.lua"
cp "$PASS_RANK_BIN" "$DEFAULTS_PATH/pass/pass_rank"
chmod 755 "$DEFAULTS_PATH/pass/pass_rank"

cat > "$CONTENTS_PATH/Info.plist" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleDevelopmentRegion</key>
  <string>en</string>
  <key>CFBundleDisplayName</key>
  <string>${APP_NAME}</string>
  <key>CFBundleExecutable</key>
  <string>runx</string>
  <key>CFBundleIdentifier</key>
  <string>${APP_BUNDLE_ID}</string>
  <key>CFBundleInfoDictionaryVersion</key>
  <string>6.0</string>
  <key>CFBundleName</key>
  <string>${APP_NAME}</string>
  <key>CFBundlePackageType</key>
  <string>APPL</string>
  <key>CFBundleShortVersionString</key>
  <string>${APP_VERSION}</string>
  <key>CFBundleVersion</key>
  <string>${APP_VERSION}</string>
  <key>LSUIElement</key>
  <true/>
  <key>NSAppleEventsUsageDescription</key>
  <string>Runx needs Apple Events access to focus windows and type into other apps.</string>
  <key>NSHighResolutionCapable</key>
  <true/>
</dict>
</plist>
EOF

printf 'Created %s\n' "$BUNDLE_PATH"
