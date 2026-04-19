local function starts_with(value, prefix)
  return value:sub(1, #prefix) == prefix
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
  local entries = runx.list_password_store()
  local items = {}
  local trimmed = query:gsub("^%s+", ""):gsub("%s+$", "")

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
      local text = runx.pass_show(action.entry)
      return runx.type_text(text)
    end

    if action.kind == "copy_password" then
      return runx.pass_copy(action.entry)
    end

    if action.kind == "type_otp" then
      local text = runx.pass_otp(action.entry)
      return runx.type_text(text)
    end

    if action.kind == "copy_otp" then
      return runx.pass_otp_copy(action.entry)
    end

    if action.kind == "generate_type" then
      local text = runx.pass_generate(action.args)
      return runx.type_text(text)
    end

    if action.kind == "generate_copy" then
      return runx.pass_generate_copy(action.args)
    end

    error("unknown pass action: " .. tostring(action.kind))
  end,
}
