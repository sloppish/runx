---@meta

---@class Runx
---@field api_version integer
---@field plugin_path string
---@field plugin_dir string
---@field plugin_config table<string, any>
runx = {}

---@param candidate string
---@param query string
---@return integer
function runx.fuzzy_score(candidate, query) end

---@param name string
---@return string?
function runx.getenv(name) end

---@param raw string
---@return string[]
function runx.parse_args(raw) end

---@param root string
---@return string[]
function runx.walk_files(root) end

---@param path string
---@return string
function runx.read_text(path) end

---@param cmd string
---@param args string[]
---@param first_line? boolean
---@param trim? boolean
---@return string
function runx.exec_capture(cmd, args, first_line, trim) end

---@param cmd string
---@param args string[]
---@param silence_stderr? boolean
---@return true
function runx.exec_status(cmd, args, silence_stderr) end

---@param cmd string
---@param args string[]
---@return any
function runx.exec_json(cmd, args) end

---@param text string
---@return any
function runx.json_decode(text) end

---@param key string
---@param value any
---@return true
function runx.session_set(key, value) end

---@param key string
---@return any?
function runx.session_get(key) end

---@param text string
function runx.copy_text(text) end

---@return string
function runx.clipboard_text() end

---@param text string
function runx.type_text(text) end

---@return string
function runx.home_dir() end

---@class RunxRunningApp
---@field pid integer
---@field name string
---@field bundle_id string?
---@field path string?

---@return RunxRunningApp[]
function runx.running_apps() end

---@param bundle_id string
---@return string?
function runx.icon_for_bundle_id(bundle_id) end

---@class RunxWindow
---@field window_id integer
---@field title string
---@field subrole string

---@param pid integer
---@return RunxWindow[]
function runx.windows_for_pid(pid) end

---@class RunxFocusWindowOptions
---@field pid integer
---@field window_id integer
---@field app_name? string
---@field title? string
---@field window_title? string
---@field all_windows? boolean

---@param options RunxFocusWindowOptions
---@return string
function runx.focus_window(options) end
