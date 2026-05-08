default:
    @just --list --unsorted

build: build-ui build-rust
test: test-ui test-rust
lint: lint-ui lint-rust
check: lint test
fmt: fmt-ui fmt-rust
run: build-ui
    cargo run

build-ui:
    @if command -v redo-ifchange >/dev/null 2>&1; then redo-ifchange frontend; else bun ui/build.ts; fi
test-ui:
    bun test ui/
lint-ui:
    bunx biome check
    bunx tsc --noEmit
fmt-ui:
    bunx biome check --write

build-rust:
    cargo build
test-rust: build-ui
    cargo test
lint-rust: build-ui
    cargo fmt --check
    cargo clippy --all-targets --all-features -- -D warnings
fmt-rust:
    cargo fmt

docs:
    cargo run --quiet --manifest-path tools/config-docgen/Cargo.toml
docs-check:
    cargo run --quiet --manifest-path tools/config-docgen/Cargo.toml -- --check

install:
    ./scripts/install-macos.sh
open: install
    open -a Runx
package:
    ./scripts/package-macos.sh
dmg:
    ./scripts/build-dmg-macos.sh --universal --ad-hoc-sign
