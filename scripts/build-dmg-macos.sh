#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
APP_NAME="Runx"
PROFILE="release"
OUT_DIR="$ROOT_DIR/dist"
VOLUME_NAME="Runx"
DMG_NAME=""
SIGN_IDENTITY="${RUNX_CODESIGN_IDENTITY:-}"
FORCE_AD_HOC_SIGN=0
NOTARIZE=0
UNIVERSAL=0
NOTARY_APPLE_ID="${RUNX_NOTARY_APPLE_ID:-}"
NOTARY_PASSWORD="${RUNX_NOTARY_PASSWORD:-}"
NOTARY_TEAM_ID="${RUNX_NOTARY_TEAM_ID:-}"

usage() {
  cat <<'EOF'
Usage: scripts/build-dmg-macos.sh [options]

Options:
  --out-dir PATH         Output directory for the app bundle and DMG (default: ./dist)
  --debug                Build with the debug profile instead of release
  --universal            Build a universal app bundle before creating the DMG
  --volume-name NAME     Finder volume name inside the DMG (default: Runx)
  --dmg-name NAME        DMG file name (default: Runx-<version>-macos.dmg)
  --sign-identity NAME   Code-signing identity to use
                         (default: RUNX_CODESIGN_IDENTITY, then Developer ID Application, then Apple Development, then ad-hoc)
  --ad-hoc-sign          Force ad-hoc signing even if a real identity is available
  --notarize             Submit the DMG to Apple notarization and staple the ticket
  --apple-id ID          Apple ID for notarization (default: RUNX_NOTARY_APPLE_ID)
  --password VALUE       App-specific password for notarization (default: RUNX_NOTARY_PASSWORD)
  --team-id TEAM         Apple team ID for notarization (default: RUNX_NOTARY_TEAM_ID)
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
    --volume-name)
      VOLUME_NAME="$2"
      shift 2
      ;;
    --dmg-name)
      DMG_NAME="$2"
      shift 2
      ;;
    --sign-identity)
      SIGN_IDENTITY="$2"
      shift 2
      ;;
    --ad-hoc-sign)
      FORCE_AD_HOC_SIGN=1
      shift
      ;;
    --notarize)
      NOTARIZE=1
      shift
      ;;
    --apple-id)
      NOTARY_APPLE_ID="$2"
      shift 2
      ;;
    --password)
      NOTARY_PASSWORD="$2"
      shift 2
      ;;
    --team-id)
      NOTARY_TEAM_ID="$2"
      shift 2
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

APP_VERSION="$(awk -F '\"' '/^version = / { print $2; exit }' "$ROOT_DIR/Cargo.toml")"
if [[ -z "$DMG_NAME" ]]; then
  DMG_NAME="${APP_NAME}-${APP_VERSION}-macos.dmg"
fi

PACKAGE_ARGS=("--out-dir" "$OUT_DIR")
if [[ "$PROFILE" == "debug" ]]; then
  PACKAGE_ARGS+=("--debug")
fi
if [[ "$UNIVERSAL" -eq 1 ]]; then
  PACKAGE_ARGS+=("--universal")
fi
if [[ "$SIGN_IDENTITY" == "-" ]]; then
  PACKAGE_ARGS+=("--ad-hoc-sign")
else
  PACKAGE_ARGS+=("--sign-identity" "$SIGN_IDENTITY")
fi

"$ROOT_DIR/scripts/package-macos.sh" "${PACKAGE_ARGS[@]}"

APP_PATH="$OUT_DIR/${APP_NAME}.app"
DMG_PATH="$OUT_DIR/$DMG_NAME"

work_dir="$(mktemp -d)"
trap 'rm -rf "$work_dir"' EXIT

notarize_artifact() {
  local artifact_path="$1"
  local label="$2"
  local notary_output
  local notary_status
  local submission_id

  set +e
  notary_output="$(
    xcrun notarytool submit "$artifact_path" \
      --apple-id "$NOTARY_APPLE_ID" \
      --password "$NOTARY_PASSWORD" \
      --team-id "$NOTARY_TEAM_ID" \
      --wait \
      --output-format json 2>&1
  )"
  notary_status=$?
  set -e

  printf '%s\n' "$notary_output"

  if [[ "$notary_status" -ne 0 ]]; then
    submission_id="$(
      printf '%s\n' "$notary_output" \
        | sed -n 's/.*"id"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' \
        | head -n 1
    )"
    if [[ -n "$submission_id" ]]; then
      printf '\nFetching notary log for %s submission %s\n' "$label" "$submission_id" >&2
      xcrun notarytool log "$submission_id" \
        --apple-id "$NOTARY_APPLE_ID" \
        --password "$NOTARY_PASSWORD" \
        --team-id "$NOTARY_TEAM_ID" || true
    fi
    exit "$notary_status"
  fi
}

if [[ "$NOTARIZE" -eq 1 ]]; then
  if [[ "$SIGN_IDENTITY" == "-" ]]; then
    echo "Cannot notarize an ad-hoc signed build. Provide a real signing identity." >&2
    exit 1
  fi
  if [[ -z "$NOTARY_APPLE_ID" || -z "$NOTARY_PASSWORD" || -z "$NOTARY_TEAM_ID" ]]; then
    echo "Notarization requires --apple-id, --password, and --team-id (or RUNX_NOTARY_* env vars)." >&2
    exit 1
  fi

  app_zip="$work_dir/${APP_NAME}.app.zip"
  ditto -c -k --keepParent "$APP_PATH" "$app_zip"
  notarize_artifact "$app_zip" "${APP_NAME}.app"
  xcrun stapler staple "$APP_PATH" >/dev/null
fi

stage_dir="$work_dir/stage"
mkdir -p "$stage_dir"
ditto "$APP_PATH" "$stage_dir/${APP_NAME}.app"
ln -s /Applications "$stage_dir/Applications"

rm -f "$DMG_PATH"
hdiutil create \
  -volname "$VOLUME_NAME" \
  -srcfolder "$stage_dir" \
  -ov \
  -format UDZO \
  "$DMG_PATH" >/dev/null

if [[ "$SIGN_IDENTITY" != "-" ]]; then
  codesign --force --timestamp --sign "$SIGN_IDENTITY" "$DMG_PATH" >/dev/null
fi

if [[ "$NOTARIZE" -eq 1 ]]; then
  notarize_artifact "$DMG_PATH" "$DMG_NAME"
  xcrun stapler staple "$DMG_PATH" >/dev/null
fi

printf 'Created %s\n' "$DMG_PATH"
if [[ "$SIGN_IDENTITY" == "-" ]]; then
  printf 'Signed with ad-hoc identity\n'
else
  printf 'Signed with %s\n' "$SIGN_IDENTITY"
fi
if [[ "$NOTARIZE" -eq 1 ]]; then
  printf 'Notarized and stapled\n'
fi
