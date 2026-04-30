# Plugin API

Runx plugins are small Lua files that return a table. They can contribute search results, implement command-routed actions, and run local commands through a narrow `runx.*` runtime API.

You do not need a manifest, a build step, or a plugin framework. A plugin is just Lua plus optional `[plugin.<id>]` config.

This page is the reference for writing plugins. For user-facing plugin configuration, see [CONFIGURATION.md](./CONFIGURATION.md).

## Where Plugins Live

By default, Runx scans:

- `~/Library/Application Support/runx/plugins/`

You can add more plugin directories through `[plugins].directories` in `config.toml`.

Runx loads every `.lua` file in those directories. One file is one plugin.

Runx evaluates the plugin file in a fresh Lua state on each search or action call. Do not rely on mutable global state surviving between invocations.

## Minimal Plugin

```lua
return {
  id = "hello",
  name = "Hello",
  badge = "HI",

  search = function(query)
    if query ~= "hello" then
      return {}
    end

    return {
      {
        title = "Say hello",
        subtitle = "Example plugin action",
        action = {
          kind = "hello",
        },
      },
    }
  end,

  run = function(action)
    if action.kind == "hello" then
      return "Hello from Runx"
    end

    error("unknown action: " .. tostring(action.kind))
  end,
}
```

## Complete Routed Example

This is the smallest realistic command-routed plugin shape: one Lua file plus one config block.

```lua
return {
  id = "hello",
  name = "Hello",
  badge = "HI",

  search_hello = function(raw, argv)
    if raw == "" then
      return {
        {
          title = "hello <name>",
          subtitle = "Type a name after the command",
          style = "full",
          action = { kind = "noop" },
        },
      }
    end

    return {
      {
        title = "Say hello to " .. raw,
        subtitle = table.concat(argv, ", "),
        style = "full",
        action = {
          kind = "hello",
          target = raw,
        },
      },
    }
  end,

  run = function(action)
    if action.kind == "noop" then
      return "Try `hello world`."
    end

    if action.kind == "hello" then
      return "Hello, " .. action.target
    end

    error("unknown action: " .. tostring(action.kind))
  end,
}
```

```toml
[plugin.hello.commands]
hello = "search_hello"
```

Typing `hello world` now routes to `search_hello(raw, argv)`.

## Plugin Table

Runx reads these top-level fields from the returned Lua table:

| Key | Required | Meaning |
| --- | --- | --- |
| `id` | no | Stable plugin id. Defaults to the file stem. |
| `name` | no | Display name. Defaults to the file stem. |
| `badge` | no | Default badge for full-style items. Defaults to `PLG`. |
| `search` | no | Generic search entrypoint for normal queries. |
| `run` | needed for actionable results | Action handler called when a plugin item is activated. |
| `search_*` | optional | Named routed handlers used through `[plugin.<id>.commands]`. |

## Search Modes

Runx supports two plugin search modes.

### Generic search

If a plugin exports `search(query)`, Runx may call it during normal provider fan-out.

Signature:

```lua
search = function(query) -> { items... }
```

Use this for plugins that behave like a normal search source.

Return `{}` when there are no results.

### Routed commands

If you configure `[plugin.<id>.commands]`, Runx routes matching query prefixes to named Lua handlers.

Example config:

```toml
[plugin.calc.commands]
"=" = "search_calc"

[plugin.emoji.commands]
emoji = "search_emoji"
copy-emoji = "search_copy_emoji"
```

Handler signature:

```lua
search_calc = function(raw, argv) -> { items... }
```

- `raw` is the unparsed remainder after the command prefix
- `argv` is a shell-like parsed argument array
- handlers should return `{}` when there are no results

Route matching rules:

- `command` matches exactly
- or `command` followed by a space and more input

Important:

- once a plugin has any configured command routes, Runx stops calling its generic `search(query)` function
- routed plugins are routed-only

## Search Result Items

Each handler returns an array of items. Every item must include:

| Key | Required | Meaning |
| --- | --- | --- |
| `title` | yes | Non-empty visible title. |
| `action` | yes | Payload passed back to `run(action)`. |

Optional keys:

| Key | Type | Default |
| --- | --- | --- |
| `id` | string | `plugin:<plugin-id>:<title>` |
| `subtitle` | string | empty for compact items, plugin name for full items |
| `score` | integer | `0` |
| `badge` | string | empty for compact items, plugin badge for full items |
| `icon` | string (URL) | none |
| `style` | `compact` or `full` | `compact` |

Notes:

- `title`, `id`, and `badge` must not be empty after trimming
- `style` must be exactly `compact` or `full`
- `icon` replaces the badge with an image; only rendered for `full`-style items
- `icon` accepts any URL the webview can load: `https://`, `file:///`, or `data:` URIs
- item validation is strict; invalid plugin items fail the search/action path instead of being silently ignored

### Compact vs full

`compact` is the default row style. Use `full` when the item needs a subtitle and a visible badge.

Example:

```lua
{
  id = "calc:" .. expression,
  title = result,
  subtitle = expression,
  style = "full",
  badge = "CALC",
  score = 1000,
  action = {
    kind = "copy_result",
    result = result,
  },
}
```

## Action Payloads

`action` is a small JSON-like table passed back into Rust and then into `run(action)`.

Required field:

| Key | Meaning |
| --- | --- |
| `kind` | Required action discriminator. |

Rules:

- `kind` must be non-empty
- `kind` must not have leading or trailing whitespace
- extra keys are allowed
- extra keys must not use the reserved key `kind`
- field names must not be empty

Example:

```lua
action = {
  kind = "copy_password",
  entry = "mail/example.com",
}
```

## `run(action)`

When the user activates a plugin result, Runx calls:

```lua
run = function(action) ... end
```

`action` contains the validated payload you returned from the item.

Return behavior:

| Return value | Result |
| --- | --- |
| `nil` | Runx shows `Ran <plugin name>` |
| non-empty string | Runx shows that string |
| `{ message = "..." }` | Runx shows that message |
| empty string or `{ message = nil }` | no message |

Example:

```lua
run = function(action)
  if action.kind == "copy_result" then
    runx.copy_text(action.result)
    return "Copied result"
  end

  error("unknown action: " .. tostring(action.kind))
end
```

## Runtime Helpers

Runx exposes a `runx` table inside the Lua VM.

| Helper | Signature | Meaning |
| --- | --- | --- |
| `runx.api_version` | number | Current plugin API version. |
| `runx.fuzzy_score` | `(candidate, query)` | Same fuzzy scorer Runx uses internally. |
| `runx.getenv` | `(name)` | Read an environment variable, or `nil`. |
| `runx.parse_args` | `(raw)` | Parse a shell-like string into an argv array. |
| `runx.walk_files` | `(root)` | Recursively list files under `root`. |
| `runx.read_text` | `(path)` | Read a UTF-8 text file. |
| `runx.exec_capture` | `(program, args, first_line_only?)` | Run a subprocess and capture stdout. |
| `runx.exec_status` | `(program, args, silence_stderr?)` | Run a subprocess and require success. |
| `runx.exec_json` | `(program, args)` | Run a subprocess, parse stdout as JSON, and return Lua data. |
| `runx.copy_text` | `(text)` | Copy text to the clipboard. |
| `runx.type_text` | `(text)` | Type text into the previously active app. |
| `runx.home_dir` | `()` | Return the current user’s home directory. |
| `runx.plugin_path` | string | Absolute path to the current plugin file. |
| `runx.plugin_dir` | string | Directory containing the current plugin file. |
| `runx.plugin_config` | table | Config from `[plugin.<id>]`, excluding the reserved `commands` table. |

Notes:

- plugin subprocess helpers inherit the normal `PATH` plus `[plugins].search_paths`
- `runx.exec_json` expects stdout to be valid JSON
- `runx.type_text` depends on the usual macOS Accessibility flow

## Configuring Plugins

Plugin-specific settings live under `[plugin.<id>]`.

Example:

```toml
[plugin.pass]
pass_rank_bin = "/Users/you/bin/pass-rank"

[plugin.pass.commands]
pass = "search_type_password"
copy-pass = "search_copy_password"
otp = "search_type_otp"
copy-otp = "search_copy_otp"
```

Inside Lua, the plugin sees:

```lua
local config = runx.plugin_config or {}
local path = config.pass_rank_bin
```

The reserved `[plugin.<id>.commands]` subtable is used only by Runx for routing and is not included in `runx.plugin_config`.

## Patterns From The Example Plugins

The bundled examples show the intended size and style:

- [examples/calc.lua](./examples/calc.lua): routed calculator, returns one full-style item and copies the result
- [examples/emoji.lua](./examples/emoji.lua): routed emoji search with `copy_text` and `type_text`
- [examples/pass.lua](./examples/pass.lua): command-routed password store integration with structured action kinds

They are good starting points if you want to copy a real plugin shape instead of starting from a blank file.
