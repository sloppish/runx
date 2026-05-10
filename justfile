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
docs-site:
    cd site && bunx vitepress dev
docs-site-build:
    cd site && bunx vitepress build
docs-site-preview:
    cd site && bunx vitepress preview

install:
    ./scripts/install-macos.sh
open: install
    open -a Runx
package:
    ./scripts/package-macos.sh
dmg:
    ./scripts/build-dmg-macos.sh --universal --ad-hoc-sign


vm_host := "macos"
vm_dir := "~/src/runx-test"
ssh_opts := "-o WarnWeakCrypto=no-pq-kex"

_sync-remote:
    ssh {{ssh_opts}} {{vm_host}} 'mkdir -p {{vm_dir}}'
    rsync -az --delete --exclude='.git/' --exclude='target/' -e 'ssh {{ssh_opts}}' ./ {{vm_host}}:{{vm_dir}}/

run-remote +command: _sync-remote
    ssh {{ssh_opts}} {{vm_host}} 'zsh -lc "source ~/.cargo/env 2>/dev/null || true; cd {{vm_dir}} && {{command}}"'

test-vm: _sync-remote
    ssh {{ssh_opts}} {{vm_host}} 'zsh -lc "source ~/.cargo/env 2>/dev/null || true; cd {{vm_dir}} && cargo test"'

lint-remote: _sync-remote
    ssh {{ssh_opts}} {{vm_host}} 'zsh -lc "source ~/.cargo/env 2>/dev/null || true; cd {{vm_dir}} && cargo fmt --check && cargo clippy --all-targets -- -D warnings"'
