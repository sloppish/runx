local function starts_with(value, prefix)
  return value:sub(1, #prefix) == prefix
end

local function trim(value)
  return value:gsub("^%s+", ""):gsub("%s+$", "")
end

local function split_args(raw)
  local parts = {}
  for part in raw:gmatch("%S+") do
    table.insert(parts, part)
  end
  return parts
end

local function first_non_flag(parts)
  for _, value in ipairs(parts) do
    if not starts_with(value, "-") then
      return value
    end
  end
  return nil
end

local function password_store_dir()
  return runx.getenv("PASSWORD_STORE_DIR") or (runx.home_dir() .. "/.password-store")
end

local function list_password_store()
  local root = password_store_dir()
  local entries = {}

  for _, path in ipairs(runx.walk_files(root)) do
    if path:sub(-4) == ".gpg" then
      table.insert(entries, path:sub(1, -5))
    end
  end

  table.sort(entries)
  return entries
end

local function pass_show(entry)
  return runx.exec_capture("pass", { "show", entry }, true)
end

local function pass_copy(entry)
  runx.exec_status("pass", { "show", "-c1", entry }, false)
  return "Copied password"
end

local function pass_otp(entry)
  return runx.exec_capture("pass", { "otp", entry }, true)
end

local function pass_otp_copy(entry)
  runx.exec_status("pass", { "otp", "-c", entry }, false)
  return "Copied OTP"
end

local function pass_generate(args)
  local parts = split_args(args)
  if #parts == 0 then
    error("pass-gen needs arguments")
  end

  local command = { "generate" }
  for _, part in ipairs(parts) do
    table.insert(command, part)
  end

  runx.exec_status("pass", command, false)

  local entry = first_non_flag(parts)
  if not entry then
    error("could not infer generated pass entry")
  end

  return pass_show(entry)
end

local function pass_generate_copy(args)
  local parts = split_args(args)
  if #parts == 0 then
    error("pass-gen-copy needs arguments")
  end

  local command = { "generate", "-c" }
  for _, part in ipairs(parts) do
    table.insert(command, part)
  end

  runx.exec_status("pass", command, false)
  return "Generated password and copied it"
end

local commands = {
  {
    trigger = "pass ",
    action = "type_password",
    badge = "PASS",
    subtitle = "Type password into the focused app",
  },
  {
    trigger = "pass-copy ",
    action = "copy_password",
    badge = "PASS",
    subtitle = "Copy password to the clipboard",
  },
  {
    trigger = "pass-otp ",
    action = "type_otp",
    badge = "OTP",
    subtitle = "Type the one-time password",
  },
  {
    trigger = "pass-otp-copy ",
    action = "copy_otp",
    badge = "OTP",
    subtitle = "Copy the one-time password",
  },
  {
    trigger = "pass-gen ",
    action = "generate_type",
    badge = "GEN",
    subtitle = "Run `pass generate ...` and type the result",
    raw = true,
  },
  {
    trigger = "pass-gen-copy ",
    action = "generate_copy",
    badge = "GEN",
    subtitle = "Run `pass generate -c ...`",
    raw = true,
  },
}

local function match_command(query)
  for _, command in ipairs(commands) do
    if query == command.trigger:sub(1, -2) or starts_with(query, command.trigger) then
      return command, query:sub(#command.trigger + 1)
    end
  end
  return nil, nil
end

local function score_entries(query, command)
  local entries = list_password_store()
  local items = {}
  local trimmed = trim(query)

  for _, entry in ipairs(entries) do
    local score = trimmed == "" and 1 or runx.fuzzy_score(entry, trimmed)
    if score > 0 then
      table.insert(items, {
        id = command.action .. ":" .. entry,
        title = entry,
        subtitle = command.subtitle,
        badge = command.badge,
        score = score,
        action = {
          kind = command.action,
          entry = entry,
        },
      })
    end
  end

  table.sort(items, function(left, right)
    if left.score == right.score then
      return left.title < right.title
    end
    return left.score > right.score
  end)

  return items
end

return {
  id = "pass",
  name = "pass",
  badge = "PASS",

  search = function(query)
    local command, payload = match_command(query)
    if not command then
      return {}
    end

    if command.raw then
      local trimmed = payload:gsub("^%s+", ""):gsub("%s+$", "")
      if trimmed == "" then
        return {
          {
            id = command.action .. ":hint",
            title = command.trigger .. "<args>",
            subtitle = "Pass the usual `pass generate` arguments. Add `-f` if overwrite is expected.",
            badge = command.badge,
            score = 10,
            action = {
              kind = "noop",
            },
          },
        }
      end

      return {
        {
          id = command.action .. ":" .. trimmed,
          title = command.trigger .. trimmed,
          subtitle = command.subtitle,
          badge = command.badge,
          score = 1000,
          action = {
            kind = command.action,
            args = trimmed,
          },
        },
      }
    end

    return score_entries(payload, command)
  end,

  run = function(action)
    if action.kind == "noop" then
      return "Pass generator is ready."
    end

    if action.kind == "type_password" then
      local text = pass_show(action.entry)
      return runx.type_text(text)
    end

    if action.kind == "copy_password" then
      return pass_copy(action.entry)
    end

    if action.kind == "type_otp" then
      local text = pass_otp(action.entry)
      return runx.type_text(text)
    end

    if action.kind == "copy_otp" then
      return pass_otp_copy(action.entry)
    end

    if action.kind == "generate_type" then
      local text = pass_generate(action.args)
      return runx.type_text(text)
    end

    if action.kind == "generate_copy" then
      return pass_generate_copy(action.args)
    end

    error("unknown pass action: " .. tostring(action.kind))
  end,
}
