#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

cd "$ROOT_DIR"
cargo run --quiet --manifest-path "$ROOT_DIR/tools/config-docgen/Cargo.toml" -- "$@"
