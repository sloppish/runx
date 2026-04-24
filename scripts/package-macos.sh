#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
APP_NAME="Runx"
APP_BUNDLE_ID="io.github.sloppish.runx"
PROFILE="release"
OUT_DIR="$ROOT_DIR/dist"
APP_ICON_SOURCE="$ROOT_DIR/assets/runx-app-icon.svg"
TRAY_ICON_SOURCE="$ROOT_DIR/assets/runx-status-template.svg"
UNIVERSAL=0

usage() {
  cat <<'EOF'
Usage: scripts/package-macos.sh [options]

Options:
  --out-dir PATH         Bundle output directory (default: ./dist)
  --debug                Build with the debug profile instead of release
  --universal            Build and package a universal binary for arm64 + x86_64 macOS
  --sign-identity NAME   Code-signing identity to use
                         (default: RUNX_CODESIGN_IDENTITY, then first Apple Development identity, then ad-hoc)
  --ad-hoc-sign          Force ad-hoc signing even if a real identity is available
  -h, --help             Show this help
EOF
}

pick_default_sign_identity() {
  local identities
  local identity
  identities="$(security find-identity -p codesigning -v 2>/dev/null || true)"

  identity="$(
    printf '%s\n' "$identities" \
      | sed -n 's/.*"Developer ID Application: \(.*\)"/Developer ID Application: \1/p' \
      | head -n 1
  )"
  if [[ -n "$identity" ]]; then
    printf '%s\n' "$identity"
    return 0
  fi

  printf '%s\n' "$identities" \
    | sed -n 's/.*"Apple Development: \(.*\)"/Apple Development: \1/p' \
    | head -n 1
}

SIGN_IDENTITY="${RUNX_CODESIGN_IDENTITY:-}"
FORCE_AD_HOC_SIGN=0

while [[ $# -gt 0 ]]; do
  case "$1" in
    --out-dir)
      OUT_DIR="$2"
      shift 2
      ;;
    --debug)
      PROFILE="debug"
      shift
      ;;
    --universal)
      UNIVERSAL=1
      shift
      ;;
    --sign-identity)
      SIGN_IDENTITY="$2"
      shift 2
      ;;
    --ad-hoc-sign)
      FORCE_AD_HOC_SIGN=1
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

if [[ "$FORCE_AD_HOC_SIGN" -eq 1 ]]; then
  SIGN_IDENTITY="-"
elif [[ -z "$SIGN_IDENTITY" ]]; then
  SIGN_IDENTITY="$(pick_default_sign_identity)"
  if [[ -z "$SIGN_IDENTITY" ]]; then
    SIGN_IDENTITY="-"
  fi
fi

render_png() {
  local source="$1"
  local output="$2"
  local size="$3"

  if [[ "$source" == *.svg ]] && command -v rsvg-convert >/dev/null 2>&1; then
    rm -f "$output"
    rsvg-convert -a -w "$size" -h "$size" "$source" -o "$output" >/dev/null 2>&1 || true
    if [[ -s "$output" ]]; then
      return 0
    fi
  fi

  rm -f "$output"
  sips -Z "$size" -s format png "$source" --out "$output" >/dev/null 2>&1 || true
  if [[ -s "$output" ]]; then
    return 0
  fi

  echo "Failed to render PNG from $source" >&2
  return 1
}

build_app_icon() {
  local resources_path="$1"
  local work_dir
  work_dir="$(mktemp -d)"
  trap 'rm -rf "$work_dir"' RETURN

  local master_png="$work_dir/runx-app-icon.png"
  render_png "$APP_ICON_SOURCE" "$master_png" 1024

  local iconset="$work_dir/Runx.iconset"
  mkdir -p "$iconset"

  sips -z 16 16     "$master_png" --out "$iconset/icon_16x16.png" >/dev/null
  sips -z 32 32     "$master_png" --out "$iconset/icon_16x16@2x.png" >/dev/null
  sips -z 32 32     "$master_png" --out "$iconset/icon_32x32.png" >/dev/null
  sips -z 64 64     "$master_png" --out "$iconset/icon_32x32@2x.png" >/dev/null
  sips -z 128 128   "$master_png" --out "$iconset/icon_128x128.png" >/dev/null
  sips -z 256 256   "$master_png" --out "$iconset/icon_128x128@2x.png" >/dev/null
  sips -z 256 256   "$master_png" --out "$iconset/icon_256x256.png" >/dev/null
  sips -z 512 512   "$master_png" --out "$iconset/icon_256x256@2x.png" >/dev/null
  sips -z 512 512   "$master_png" --out "$iconset/icon_512x512.png" >/dev/null
  cp "$master_png" "$iconset/icon_512x512@2x.png"

  iconutil -c icns "$iconset" -o "$resources_path/Runx.icns"
}

build_tray_icon() {
  local resources_path="$1"
  render_png "$TRAY_ICON_SOURCE" "$resources_path/RunxStatusTemplate.png" 36
}

APP_VERSION="$(awk -F '"' '/^version = / { print $2; exit }' "$ROOT_DIR/Cargo.toml")"
TARGET_DIR="$(
  cargo metadata --format-version 1 --no-deps --manifest-path "$ROOT_DIR/Cargo.toml" \
    | plutil -extract target_directory raw -o - -
)"

if [[ -z "$TARGET_DIR" ]]; then
  echo "Failed to resolve Cargo target directory" >&2
  exit 1
fi

if [[ "$UNIVERSAL" -eq 1 ]]; then
  targets=(aarch64-apple-darwin x86_64-apple-darwin)
  bin_paths=()
  for target in "${targets[@]}"; do
    cargo_args=(build --manifest-path "$ROOT_DIR/Cargo.toml" --target "$target")
    if [[ "${PROFILE}" == "release" ]]; then
      cargo_args+=(--release)
    fi
    cargo "${cargo_args[@]}"
    bin_paths+=("$TARGET_DIR/$target/$PROFILE/runx")
  done

  UNIVERSAL_BIN_DIR="$TARGET_DIR/universal/$PROFILE"
  mkdir -p "$UNIVERSAL_BIN_DIR"
  BIN_PATH="$UNIVERSAL_BIN_DIR/runx"
  lipo -create "${bin_paths[@]}" -output "$BIN_PATH"
else
  if [[ "${PROFILE}" == "release" ]]; then
    cargo build --release --manifest-path "$ROOT_DIR/Cargo.toml"
  else
    cargo build --manifest-path "$ROOT_DIR/Cargo.toml"
  fi
  BIN_PATH="$TARGET_DIR/$PROFILE/runx"
fi

BUNDLE_PATH="$OUT_DIR/${APP_NAME}.app"
CONTENTS_PATH="$BUNDLE_PATH/Contents"
MACOS_PATH="$CONTENTS_PATH/MacOS"
RESOURCES_PATH="$CONTENTS_PATH/Resources"

rm -rf "$BUNDLE_PATH"
mkdir -p "$MACOS_PATH" "$RESOURCES_PATH"

cp "$BIN_PATH" "$MACOS_PATH/runx"
chmod 755 "$MACOS_PATH/runx"
build_app_icon "$RESOURCES_PATH"
build_tray_icon "$RESOURCES_PATH"

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
  <key>CFBundleIconFile</key>
  <string>Runx.icns</string>
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
  <string>Runx plugins may use Apple Events to automate other apps.</string>
  <key>NSHighResolutionCapable</key>
  <true/>
</dict>
</plist>
EOF

codesign \
  --force \
  --deep \
  --options runtime \
  --timestamp \
  --sign "$SIGN_IDENTITY" \
  --identifier "$APP_BUNDLE_ID" \
  "$BUNDLE_PATH" >/dev/null

printf 'Created %s\n' "$BUNDLE_PATH"
if [[ "$SIGN_IDENTITY" == "-" ]]; then
  printf 'Signed with ad-hoc identity\n'
else
  printf 'Signed with %s\n' "$SIGN_IDENTITY"
fi
if [[ "$UNIVERSAL" -eq 1 ]]; then
  lipo -info "$MACOS_PATH/runx"
fi
