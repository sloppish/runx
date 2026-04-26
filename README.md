<p align="center">
  <img src="assets/runx-app-icon.svg" width="140" alt="Runx icon">
</p>

<h1 align="center">Runx</h1>

<p align="center">
  A snappy macOS launcher for switching windows, apps, settings, and more.
</p>

<p align="center">
  <a href="./CONFIGURATION.md">Configuration</a>
  ·
  <a href="./PLUGIN_API.md">Plugin API</a>
</p>

<p align="center">
  <img src="assets/readme-screenshot.webp" width="780" alt="Runx launcher showing open windows and apps">
</p>

Runx is a keyboard-first macOS launcher focused on a small core loop:

- jump to open windows
- launch installed apps
- open System Settings panes
- trigger Lua plugins and command-routed actions

It stays intentionally narrow: strict TOML config, a small Lua host API, and no attempt to be a general automation platform.

## Quick Start

```bash
cargo run
./scripts/install-macos.sh
open -a Runx
./scripts/package-macos.sh
./scripts/build-dmg-macos.sh --universal
```

## Compatibility

Runx targets macOS 11 Big Sur and newer.

## Docs

- [Configuration](./CONFIGURATION.md): generated `config.toml` reference
- [Plugin API](./PLUGIN_API.md): Lua plugin API and action payloads

## Why (or why not) Runx

| It features...                   | Which means...                                   |
| ---                              | ---                                              |
| window-first search              | no builtin web search, AI, or cloud integrations |
| macOS-tied experience            | no support for other OSes is expected            |
| a single TOML config file        | no GUI for configuration                         |
| a small host API for Lua plugins | no visual workflows builder                      |
