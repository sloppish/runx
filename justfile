default:
    @just --list

install:
    ./scripts/install-macos.sh

build-ui:
    @if command -v redo >/dev/null 2>&1; then redo frontend; else bun ui/build.ts; fi

build-rust:
    cargo build

build: build-ui build-rust

typecheck:
    bunx tsc --noEmit

test-ui:
    bun test ui/

test-rust: build-ui
    cargo test

test: test-ui test-rust

fmt-ui:
    bunx biome check --write

fmt-rust:
    cargo fmt

fmt: fmt-ui fmt-rust

lint-ui:
    bunx biome check

lint-rust: build-ui
    cargo fmt --check
    cargo clippy --all-targets --all-features -- -D warnings

lint: lint-ui lint-rust

check: lint typecheck test

run: build-ui
    cargo run

open: install
    open -a Runx

package:
    ./scripts/package-macos.sh

dmg:
    ./scripts/build-dmg-macos.sh --universal --ad-hoc-sign
