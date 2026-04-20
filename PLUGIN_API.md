# Runx Plugin API

Runx plugins are Lua files loaded from the runtime plugin directory and any extra paths configured in `config.toml`.

The current Lua runtime API version is:

```lua
runx.api_version == 1
```

Plugins should treat that as the compatibility contract for the `runx.*` helpers below.

## File Shape

A plugin file must evaluate to a Lua table. Supported top-level fields:

- `id: string?`
- `name: string?`
- `badge: string?`
- `search(query): table?`
- `<handler>(raw, argv): table?`
- `run(action): string|table|nil?`

`id`, `name`, and `badge` are optional metadata. If omitted:

- `id` falls back to the filename without `.lua`
- `name` falls back to `id`
- `badge` falls back to `"PLG"`

## Search Entry Points

`search(query)` is optional. If present, it should return an array-like Lua table of result items.

Runx also supports config-routed search handlers. In `config.toml`, you can map a command prefix to a function name inside the plugin:

```toml
[plugin.calc.commands]
calc = "search_calc"
```

When the user types `calc 2+2`, Runx strips the command prefix and calls:

```lua
search_calc("2+2", { "2+2" })
```

`raw` is the suffix after the command prefix. `argv` is the shell-like parsed argument list produced by Runx. For example:

```lua
-- user input
pass-gen 1 "2 3" '4 "5"'

-- handler arguments
raw  == [[1 "2 3" '4 "5"']]
argv == { "1", "2 3", [[4 "5"]] }
```

That lets plugin code choose the simpler representation for each command without manually checking prefixes and reparsing strings itself.

If a plugin has configured commands, Runx routes matching queries to those handlers and does not call the plugin’s legacy `search(query)` fallback for that plugin.

Compatibility note: handlers that only accept a single `raw` argument still work. Lua ignores the extra `argv` parameter if the function does not use it.

Each item must match this schema:

```lua
{
  id = "optional-stable-id",
  title = "Required title",
  subtitle = "Optional subtitle",
  badge = "Optional badge override",
  score = 123,
  action = {
    kind = "required_action_kind",
    -- any extra JSON-like fields
  },
}
```

Validation rules enforced by Runx:

- `title` is required and must not be empty after trimming
- `id`, if present, must not be empty after trimming
- `badge`, if present, must not be empty after trimming
- `action.kind` is required and must not be empty
- `action.kind` must not contain leading or trailing whitespace

Normalization rules:

- missing `id` becomes `plugin:<plugin_id>:<title>`
- missing `badge` falls back to the plugin badge
- missing or blank `subtitle` falls back to the plugin name
- missing `score` falls back to `0`

If a returned item is invalid, the plugin search fails for that query with a clear error.

## `run(action)`

`run(action)` is optional, but a plugin that returns actions from `search()` usually needs it.

Runx passes the `action` table back into `run()` exactly as JSON-compatible Lua data. The only required field is:

```lua
action.kind
```

`run(action)` may return:

- `nil`: Runx shows a generic success message
- `string`: Runx shows that message
- `{ message = "..." }`: Runx shows that message

Throwing a Lua error surfaces as an action failure in the UI.

## Runtime Helpers

Runx exposes a global `runx` table with these fields:

- `runx.api_version: integer`
- `runx.plugin_config: table`
- `runx.fuzzy_score(candidate, query) -> integer`
- `runx.getenv(name) -> string|nil`
- `runx.parse_args(raw) -> { string, ... }`
- `runx.walk_files(root) -> { string, ... }`
- `runx.read_text(path) -> string`
- `runx.exec_capture(program, args, first_line_only?) -> string`
- `runx.exec_status(program, args, silence_stderr?) -> true`
- `runx.exec_json(program, args) -> table`
- `runx.copy_text(text) -> string`
- `runx.type_text(text) -> string`
- `runx.home_dir() -> string`
- `runx.plugin_path: string`
- `runx.plugin_dir: string`

Notes:

- `runx.plugin_config` is the decoded `[plugin.<id>]` table from `config.toml`
- `exec_*` commands run with the app environment plus any configured `[plugins].search_paths`
- `type_text()` uses Runx’s current “type into previously focused app” behavior and may require macOS Accessibility permission

## Configuration

Plugin-specific config lives under:

```toml
[plugin.my_plugin]
example = "value"
```

Command routing for a plugin lives under:

```toml
[plugin.my_plugin.commands]
hello = "search_hello"
```

Extra executable lookup paths for plugin subprocesses live under:

```toml
[plugins]
search_paths = ["/opt/homebrew/bin"]
```

## Minimal Example

```lua
if (runx.api_version or 0) < 1 then
  error("Runx plugin API v1 or newer is required")
end

return {
  id = "hello",
  name = "Hello",
  badge = "HI",

  search_hello = function(raw, argv)
    return {
      {
        title = "Say hello",
        subtitle = raw,
        action = {
          kind = "hello",
          argv = argv,
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
