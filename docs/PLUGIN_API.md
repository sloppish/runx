# Plugin API

Runx plugins are small [Lua 5.5](https://www.lua.org/manual/5.5/) files that return a table. They can contribute search results, implement command-routed actions, and perform system tasks through the `runx.*` runtime API.

You do not need a manifest, a build step, or a plugin framework. A plugin is just a Lua script plus optional `[plugin.<id>]` configuration in your `config.toml`.

## Where Plugins Live

By default, Runx scans:

- `~/Library/Application Support/runx/plugins/`

You can add more plugin directories through `[plugins].directories` in `config.toml`.

- **One directory is one plugin:** Each plugin is a subdirectory containing `init.lua` as its entry point (e.g. `plugins/calc/init.lua`).
- **`require` support:** Plugins can load sibling files via `require("utils")` which resolves to `utils.lua` or `utils/init.lua` in the same directory.
- **Fresh state:** Runx evaluates the plugin in a fresh Lua state on every search or action. Do not rely on global variables surviving between calls. Garbage collection is disabled since the entire state is discarded after each call. Use `runx.session_set` / `runx.session_get` for small per-invocation caches that should survive fresh Lua evaluations while the launcher stays open.

## Editor Support

Runx writes Lua language server metadata next to your user config:

- `~/Library/Application Support/runx/.luarc.json`
- `~/Library/Application Support/runx/types/runx.lua`

Open `~/Library/Application Support/runx` as your editor workspace to get `runx.*` completions and diagnostics while editing plugins under `plugins/`.

## Minimal Plugin

A plugin must return a table containing its identity and at least one search or action handler.

```lua
return {
  id = "hello",       -- Unique identifier
  name = "Hello",     -- Display name for the UI
  badge = "HI",       -- Default badge for full-style items

  -- Generic search handler
  search = function(query)
    if query ~= "hello" then return {} end

    return {
      {
        title = "Say hello",
        subtitle = "A simple plugin example",
        payload = { kind = "greet" },
      },
    }
  end,

  -- Execution handler
  run = function(payload)
    if payload.kind == "greet" then
      return "Hello from Runx!"
    end
  end,
}
```

## Plugin Table Reference

| Key | Required | Type | Default | Meaning |
| --- | --- | --- | --- | --- |
| `id` | no | string | Directory name | Stable identifier used to match `[plugin.<id>]` config and command routes. |
| `name` | no | string | Directory name | Display name shown in the UI. |
| `badge` | no | string | `"PLG"` | Default badge shown on full-style items. |
| `commands` | no | table | - | Default command routes: `{ prefix = "handler_name" }`. |
| `aliases` | no | table | - | Default command aliases: `{ command = "alias" }`. |
| `stateless` | no | boolean | `false` | When true, in-flight searches are cancelled on each keystroke even within the same route. Set for plugins that don't use the session store. |
| `search` | no | function | - | Generic search entrypoint for normal queries. |
| `run` | no | function | - | Called when a plugin item is activated. |
| `\<handler\>` | no | function | - | Named routed handlers referenced by `commands`. |

## Search Modes

### 1. Generic Search

If a plugin exports `search(query)`, Runx calls it during normal search. Use this for plugins that should always contribute results (like a calculator that activates on numbers).

```lua
search = function(query) -> { items... }
```

### 2. Routed Commands (Recommended)

Routed commands allow you to trigger a plugin with a specific prefix (e.g., `calc 1+1`). This is faster and prevents your plugin from interfering with general search results.

Plugins can declare their own default commands by exporting a `commands` table. These are used automatically unless the user overrides them in `config.toml`:

```lua
return {
  id = "echo",
  commands = { echo = "handle_echo" },

  handle_echo = function(raw, argv)
    return {
      { title = raw, payload = { text = raw } },
    }
  end,

  run = function(payload)
    runx.copy_text(payload.text)
    return "Copied!"
  end,
}
```

Users can override a plugin's default commands in `config.toml`:

```toml
[plugin.echo.commands]
"e" = "handle_echo"
```

When `[plugin.<id>.commands]` is present in the user's config, it completely replaces the plugin's built-in `commands` table.

### Command Aliases

Plugins can declare aliases for their commands:

```lua
return {
  commands = { "pass-otp" = "search_otp" },
  aliases = { "pass-otp" = "otp" },
  ...
}
```

Each alias maps a canonical command name (key) to a shorter trigger (value). Typing `p foo` behaves identically to `pass foo` — the alias is transparent to the plugin handler.

Users can override aliases in `config.toml`:

```toml
[plugin.pass.aliases]
pass-otp = "otp"
```

When `[plugin.<id>.aliases]` is present, it completely replaces the plugin's built-in `aliases` table. An empty section disables all aliases.

Alias conflicts (two routes sharing the same trigger string) are rejected at config load time.

**Lua handler signature:**
```lua
-- raw: everything after "echo " (e.g., "hello world")
-- argv: shell-parsed arguments (e.g., {"hello", "world"})
handle_echo = function(raw, argv)
  -- return items...
end
```

**Important:** Once a plugin has command routes (either self-declared or from config), its generic `search(query)` function is **ignored**.

## Search Result Items

Handlers must return an array of item tables.

| Key | Required | Type | Default | Meaning |
| --- | --- | --- | --- | --- |
| `title` | **yes** | string | - | The main visible label. |
| `payload` | **yes** | table | - | Opaque data passed to `run(payload)` on activation. |
| `subtitle` | no | string | `""` compact / plugin name full | Secondary text (shown in `full` style). |
| `score` | no | integer | `0` | Ranking priority (higher is better). |
| `badge` | no | string | `""` compact / plugin badge full | Small text tag on the right. |
| `icon` | no | string | - | URL or `data:` URI (replaces badge). |
| `style` | no | string | `compact` | `compact` (single line) or `full` (two lines). |
| `id` | no | string | `plugin:\<plugin_id\>:\<title\>` | Stable ID for selection memory. |

### Visual Styles

- **`compact` (Default):** A slim, single-line row. Subtitles and badges are hidden unless the UI theme explicitly enables them for compact rows.
- **`full`:** A taller, two-line row. Shows the `subtitle` below the `title` and the `badge` (or `icon`) on the right.

## Payload

The `payload` table is any JSON-serializable Lua table. It is stored when your search handler returns it and passed back to `run(payload)` when the user activates the item. Runx treats it as opaque data — structure it however you like.

```lua
payload = {
  kind = "copy_password",
  account = "github.com",
}
```

## `run(payload)` and Feedback

The `run` function executes the requested action. You can return a value to control the feedback message logged by Runx (visible in `debug.log` or terminal output):

| Return Value | Logged Message |
| --- | --- |
| `nil` | "Ran \<Plugin Name\>" |
| `string` | The returned string. |
| `""` | Nothing logged. |

## Runtime Helpers (`runx.*`)

Runx provides a built-in `runx` table with utility functions. Parameters marked with `?` are optional.

### System & Environment

#### `runx.api_version`: integer

Current API version (currently `2`).

#### `runx.plugin_path`: string

Absolute path to the current `.lua` file.

#### `runx.plugin_dir`: string

Directory containing the current plugin.

#### `runx.plugin_config`: table

Your plugin's `[plugin.<id>]` config (excludes `commands`).

#### `runx.home_dir() -> string`

Returns the user's home directory path.

#### `runx.getenv(name: string) -> string?`

Returns an environment variable or `nil`.

#### `runx.running_apps() -> table[]`

Returns regular foreground-capable macOS applications currently known to the system.

Each returned app has:

- `pid`: process ID
- `name`: localized application name or bundle identifier fallback
- `bundle_id`: bundle identifier, if available
- `path`: application bundle path, if available

#### `runx.icon_for_bundle_id(bundle_id: string) -> string?`

Returns a Runx icon URL for a running application with the given bundle identifier, or `nil` when the app is unknown or the icon is still being prepared.

The returned string is a lightweight `runx://localhost/icon/...webp` URL suitable for a search item's `icon` field. The first lookup may return `nil`; Runx will refresh visible results when the icon becomes ready.

#### `runx.windows_for_pid(pid: integer) -> table[]`

Returns Accessibility-visible windows for a running application process.

Each returned window has:

- `window_id`: CoreGraphics window ID
- `title`: Accessibility window title
- `subrole`: Accessibility subrole, such as `AXStandardWindow`

Requires Accessibility permission for Runx.

#### `runx.focus_window(options: table) -> string`

Focuses a specific macOS window using the same focusing path as Runx's built-in window provider.

Required options:

- `pid`: process ID
- `window_id`: CoreGraphics window ID

Optional options:

- `app_name`: application name used in feedback messages
- `title` or `window_title`: window title used in feedback messages
- `all_windows`: when `true`, activates all windows for the app before focusing the target

Requires Accessibility permission for exact window focus. If the target cannot be focused directly, Runx falls back to activating the app and returns a feedback message.

### Files & Execution

Subprocess helpers search the system `PATH` plus any extra directories listed in `[plugins].search_paths` in your `config.toml`.

#### `runx.read_text(path: string) -> string`

Reads a UTF-8 file and returns its contents.

#### `runx.walk_files(root: string) -> string[]`

Lists all files recursively under `root`. Returns paths relative to `root`.

#### `runx.parse_args(raw: string) -> string[]`

Parses a string into an argv array using shell quoting rules.

#### `runx.exec_capture(cmd: string, args: string[], first_line?: boolean, trim?: boolean) -> string`

Runs a command and returns stdout.

- `first_line` (default `false`) — return only the first line of output.
- `trim` (default `true`) — strip leading/trailing whitespace.
- Errors if the command exits with a non-zero status.

#### `runx.exec_status(cmd: string, args: string[], silence_stderr?: boolean) -> true`

Runs a command; returns `true` on success, errors on failure.

- `silence_stderr` (default `false`) — discard stderr output.

#### `runx.exec_json(cmd: string, args: string[]) -> table`

Runs a command, parses stdout as JSON, and returns it as a Lua table. Errors if the command fails or output is not valid JSON.

#### `runx.json_decode(text: string) -> table`

Parses a JSON string and returns it as a Lua table. Errors if the input is not valid JSON.

### Session

Session values are in-memory only. They survive separate Lua evaluations while the launcher panel is open, then clear when the panel is hidden or dismissed. Keys are automatically scoped to the current plugin id, so use plugin-local names such as `"tabs:snapshot"`.

Values must be JSON-serializable plain Lua data: `nil`, booleans, numbers, strings, array-like tables, object-like tables, and nested combinations. Functions, userdata, threads, cyclic tables, metatable/object identity, and other unsupported values are rejected.

Runx keeps at most 1024 session keys per plugin and evicts the oldest key when the cap is exceeded.

#### `runx.session_set(key: string, value: any) -> true`

Stores a JSON-serializable value for the current launcher invocation.

```lua
runx.session_set("tabs:snapshot", tabs)
```

#### `runx.session_get(key: string) -> any?`

Returns a previously stored value, or `nil` if the key is missing.

```lua
local tabs = runx.session_get("tabs:snapshot")
```

### Clipboard & Interaction

#### `runx.copy_text(text: string)`

Copies text to the system clipboard.

#### `runx.clipboard_text() -> string`

Returns the current clipboard text.

#### `runx.type_text(text: string)`

Types text into the previously active app (requires Accessibility).

### Utilities

#### `runx.fuzzy_score(target: string, query: string) -> integer`

Returns the same fuzzy match score Runx uses internally.

## Example: Fuzzy-Filtered Bookmarks

A complete plugin that uses `runx.fuzzy_score` to filter and rank results:

```lua
return {
  id = "bookmarks",
  name = "Bookmarks",
  badge = "BM",

  search = function(query)
    if query == "" then return {} end

    local bookmarks = {
      { title = "GitHub", url = "https://github.com" },
      { title = "Rust Documentation", url = "https://doc.rust-lang.org" },
      { title = "Lua Reference", url = "https://www.lua.org/manual/5.5/" },
    }

    local results = {}
    for _, bm in ipairs(bookmarks) do
      local score = runx.fuzzy_score(bm.title, query)
      if score > 0 then
        results[#results + 1] = {
          title = bm.title,
          subtitle = bm.url,
          score = score,
          style = "full",
          payload = { kind = "open", url = bm.url },
        }
      end
    end

    return results
  end,

  run = function(payload)
    runx.exec_status("open", { payload.url })
    return ""
  end,
}
```

## Debugging & Errors

- **Validation Errors:** If your search handler returns an invalid item (e.g., missing `title` or invalid `kind`), Runx will show a descriptive error in the UI.
- **Lua Errors:** If your code throws an error (via `error()` or a syntax mistake), Runx captures the message and displays it as a notification.
- **Logging:** Use `print()` to send output to the Runx process `stdout`. If you run Runx from a terminal, you'll see these logs.
