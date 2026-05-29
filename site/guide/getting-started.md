# Getting Started

Runx is a native macOS launcher written in Rust. It provides keyboard-driven app launching, window switching, and an extensible plugin system.

## Installation

### Homebrew (Recommended)

```bash
brew tap sloppish/runx
brew install runx
```

### From DMG

Download the latest `.dmg` from the [GitHub Releases](https://github.com/sloppish/runx/releases) page, open it, and drag Runx to your Applications folder.


## First Launch

After installation, open Runx from your Applications folder. It will appear as a menu bar icon.

Press **Option+Space** to open the launcher, or click the menu bar icon.

## Permissions

Runx may request:

- **Accessibility**: Required for window switching and text typing
- **Screen Recording**: Optional, needed to search and switch to windows on other spaces

macOS will prompt you when needed — grant them in System Settings > Privacy & Security.

## Configuration

Right-click the menu bar icon and choose Settings to customize hotkeys, appearance, and behavior.

Alternatively, edit the config file directly at `~/Library/Application Support/runx/config.toml`. It reloads automatically when modified. See the [Configuration Reference](/guide/configuration) for all available options.

## Next Steps

- [Configuration Reference](/guide/configuration) — customize hotkeys, window behavior, themes
- [Plugin API](/guide/plugins) — extend Runx with Lua scripts
