# Plugin API

Runx plugins are small [Lua 5.5](https://www.lua.org/manual/5.5/) files that return a table. They can contribute search results, implement command-routed actions, and perform system tasks through the `runx.*` runtime API.

You do not need a manifest, a build step, or a plugin framework. A plugin is just a Lua script plus optional `[plugin.<id>]` configuration in your `config.toml`.

## Where Plugins Live

By default, Runx scans:

- `~/Library/Application Support/runx/plugins/`

You can add more plugin directories through `[plugins].directories` in `config.toml`.

- **One directory is one plugin:** Each plugin is a subdirectory containing `init.lua` as its entry point (e.g. `plugins/calc/init.lua`).
- **`require` support:** Plugins can load sibling files via `require("utils")` which resolves to `utils.lua` or `utils/init.lua` in the same directory.
- **Fresh state:** Runx evaluates the plugin in a fresh Lua state on every search or action. Do not rely on global variables surviving between calls. Garbage collection is disabled since the entire state is discarded after each call.

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
| `id` | no | string | Filename stem | Stable identifier used to match `[plugin.<id>]` config and command routes. |
| `name` | no | string | Filename stem | Display name. Defaults to the filename. |
| `badge` | no | string | `"PLG"` | Default badge shown on full-style items. |
| `search` | no | function | - | Generic search entrypoint for normal queries. |
| `run` | no | function | - | Called when a plugin item is activated. |
| `search_*` | no | function | - | Named routed handlers used via config. |

## Search Modes

### 1. Generic Search

If a plugin exports `search(query)`, Runx calls it during normal search. Use this for plugins that should always contribute results (like a calculator that activates on numbers).

```lua
search = function(query) -> { items... }
```

### 2. Routed Commands (Recommended)

Routed commands allow you to trigger a plugin with a specific prefix (e.g., `calc 1+1`). This is faster and prevents your plugin from interfering with general search results.

**Config (`config.toml`):**
```toml
[plugin.myplugin.commands]
"calc" = "search_calc"
```

**Lua:**
```lua
-- raw: everything after "calc " (e.g., "1 + 1")
-- argv: shell-parsed arguments (e.g., {"1", "+", "1"})
search_calc = function(raw, argv)
  -- return items...
end
```

**Important:** Once a plugin has configured command routes, its generic `search(query)` function is **ignored**.

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
| `id` | no | string | `plugin:<plugin_id>:<title>` | Stable ID for selection memory. |

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

The `run` function executes the requested action. You can return feedback to be shown in the Runx UI:

| Return Value | UI Feedback |
| --- | --- |
| `nil` | Shows "Ran <Plugin Name>" |
| `string` | Shows the returned string. |
| `{ message = "..." }` | Shows the specified message. |
| `""` or `{ message = nil }` | No feedback shown. |

## Runtime Helpers (`runx.*`)

Runx provides a built-in `runx` table with utility functions. Parameters marked with `?` are optional.

### System & Environment

#### `runx.api_version`: integer

Current API version (currently `1`).

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
