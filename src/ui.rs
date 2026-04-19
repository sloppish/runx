use crate::config::UiConfig;

pub fn html(theme: &UiConfig) -> String {
    format!(
        r#"<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8" />
  <meta name="viewport" content="width=device-width, initial-scale=1" />
  <style>
    :root {{
      --accent: {accent};
      --bg: {background};
      --panel: {panel};
      --text: {text};
      --muted: {muted};
      --font: {font_family};
      --shell-bg: linear-gradient(180deg, rgba(255, 255, 255, 0.96), rgba(252, 246, 238, 0.98));
      --shell-shadow: inset 0 1px 0 rgba(255, 255, 255, 0.72);
      --shell-border: rgba(140, 102, 67, 0.16);
      --label-strong: color-mix(in srgb, var(--text) 88%, white);
      --input-bg: linear-gradient(180deg, rgba(255,255,255,0.92), rgba(248,240,230,0.9));
      --input-border: rgba(140, 102, 67, 0.14);
      --input-shadow: inset 0 1px 0 rgba(255,255,255,0.72);
      --placeholder: color-mix(in srgb, var(--muted) 78%, white);
      --scrollbar: rgba(134, 98, 66, 0.18);
      --item-bg: rgba(255,255,255,0.45);
      --item-hover: rgba(255,255,255,0.86);
      --item-selected-bg: linear-gradient(135deg, rgba(199, 123, 73, 0.15), rgba(255,255,255,0.94));
      --item-selected-shadow:
        0 18px 44px rgba(139, 87, 46, 0.14),
        inset 0 0 0 1px rgba(199, 123, 73, 0.22);
      --badge-bg: linear-gradient(180deg, color-mix(in srgb, var(--accent) 18%, white), rgba(255,255,255,0.95));
      --badge-border: rgba(199, 123, 73, 0.18);
      --badge-text: color-mix(in srgb, var(--accent) 72%, black);
      --chip-text: color-mix(in srgb, var(--muted) 80%, white);
      --chip-bg: rgba(255,255,255,0.68);
      --chip-border: rgba(140, 102, 67, 0.12);
      --status-error: #9c4420;
    }}

    @media (prefers-color-scheme: dark) {{
      :root {{
        --accent: #d7a17b;
        --bg: #16110f;
        --panel: #211916;
        --text: #f3eadf;
        --muted: #a99a8c;
        --shell-bg: linear-gradient(180deg, rgba(37, 30, 27, 0.96), rgba(24, 19, 17, 0.985));
        --shell-shadow: inset 0 1px 0 rgba(255, 255, 255, 0.05);
        --shell-border: rgba(215, 161, 123, 0.16);
        --label-strong: color-mix(in srgb, var(--text) 94%, white);
        --input-bg: linear-gradient(180deg, rgba(33, 26, 23, 0.98), rgba(27, 21, 19, 0.98));
        --input-border: rgba(215, 161, 123, 0.12);
        --input-shadow: inset 0 1px 0 rgba(255,255,255,0.03);
        --placeholder: color-mix(in srgb, var(--muted) 88%, black);
        --scrollbar: rgba(215, 161, 123, 0.24);
        --item-bg: rgba(255,255,255,0.025);
        --item-hover: rgba(255,255,255,0.06);
        --item-selected-bg: linear-gradient(135deg, rgba(215, 161, 123, 0.16), rgba(41, 31, 27, 0.98));
        --item-selected-shadow:
          0 18px 44px rgba(0, 0, 0, 0.22),
          inset 0 0 0 1px rgba(215, 161, 123, 0.2);
        --badge-bg: linear-gradient(180deg, rgba(215, 161, 123, 0.14), rgba(255,255,255,0.02));
        --badge-border: rgba(215, 161, 123, 0.16);
        --badge-text: color-mix(in srgb, var(--accent) 82%, white);
        --chip-text: color-mix(in srgb, var(--muted) 86%, white);
        --chip-bg: rgba(255,255,255,0.03);
        --chip-border: rgba(215, 161, 123, 0.12);
        --status-error: #f18a63;
      }}
    }}

    * {{
      box-sizing: border-box;
    }}

    html, body {{
      width: 100%;
      height: 100%;
      margin: 0;
      overflow: hidden;
      background: transparent;
      color: var(--text);
      font-family: var(--font);
      color-scheme: light dark;
      -webkit-font-smoothing: antialiased;
      user-select: none;
    }}

    body {{
      padding: 18px;
    }}

    .shell {{
      height: 100%;
      display: flex;
      flex-direction: column;
      gap: 14px;
      padding: 18px;
      border-radius: 24px;
      overflow: hidden;
      background: var(--shell-bg);
      box-shadow: var(--shell-shadow);
      border: 1px solid var(--shell-border);
    }}

    .label {{
      display: flex;
      justify-content: space-between;
      align-items: center;
      text-transform: uppercase;
      letter-spacing: 0.14em;
      font-size: 10px;
      color: var(--muted);
    }}

    .label strong {{
      color: var(--label-strong);
      font-weight: 700;
    }}

    .input-wrap {{
      position: relative;
      border-radius: 18px;
      padding: 14px 18px;
      background: var(--input-bg);
      border: 1px solid var(--input-border);
      box-shadow: var(--input-shadow);
    }}

    input {{
      width: 100%;
      border: 0;
      outline: none;
      padding: 0;
      background: transparent;
      color: var(--text);
      font-size: 30px;
      font-weight: 600;
      letter-spacing: -0.03em;
      font-family: var(--font);
      caret-color: var(--accent);
    }}

    input::placeholder {{
      color: var(--placeholder);
    }}

    .results {{
      flex: 1;
      min-height: 0;
      display: flex;
      flex-direction: column;
      gap: 8px;
      overflow: auto;
      padding-right: 4px;
    }}

    .results::-webkit-scrollbar {{
      width: 8px;
    }}

    .results::-webkit-scrollbar-thumb {{
      background: var(--scrollbar);
      border-radius: 999px;
    }}

    .item {{
      display: grid;
      grid-template-columns: auto 1fr auto;
      gap: 14px;
      align-items: center;
      width: 100%;
      padding: 13px 14px;
      border: 0;
      border-radius: 18px;
      background: var(--item-bg);
      cursor: pointer;
      transition: background 120ms ease, box-shadow 120ms ease;
      text-align: left;
      color: inherit;
      font: inherit;
    }}

    .item:hover {{
      background: var(--item-hover);
    }}

    .item.selected {{
      background: var(--item-selected-bg);
      box-shadow: var(--item-selected-shadow);
    }}

    .badge {{
      min-width: 46px;
      height: 46px;
      padding: 0 12px;
      border-radius: 14px;
      background: var(--badge-bg);
      border: 1px solid var(--badge-border);
      display: grid;
      place-items: center;
      font-size: 11px;
      font-weight: 700;
      letter-spacing: 0.18em;
      color: var(--badge-text);
      overflow: hidden;
    }}

    .badge.has-icon {{
      min-width: 46px;
      width: 46px;
      padding: 0;
      background: rgba(255,255,255,0.66);
    }}

    .icon-image {{
      width: 100%;
      height: 100%;
      object-fit: contain;
      display: block;
      border-radius: 14px;
    }}

    .copy {{
      min-width: 0;
    }}

    .title {{
      font-size: 16px;
      font-weight: 650;
      letter-spacing: -0.02em;
      white-space: nowrap;
      overflow: hidden;
      text-overflow: ellipsis;
    }}

    .subtitle {{
      margin-top: 4px;
      font-size: 12px;
      color: var(--muted);
      white-space: nowrap;
      overflow: hidden;
      text-overflow: ellipsis;
    }}

    .accelerator {{
      color: var(--chip-text);
      font-size: 12px;
      font-weight: 600;
      padding: 6px 9px;
      border-radius: 999px;
      background: var(--chip-bg);
      border: 1px solid var(--chip-border);
    }}

    .footer {{
      display: flex;
      justify-content: space-between;
      align-items: center;
      gap: 10px;
      font-size: 12px;
      color: var(--muted);
      min-height: 18px;
    }}

    .status.error {{
      color: var(--status-error);
    }}
  </style>
</head>
<body>
  <main class="shell">
    <div class="label">
      <strong>Runx</strong>
      <span>spotlight, windows, settings, plugins</span>
    </div>
    <div class="input-wrap">
      <input id="query" type="text" autocomplete="off" spellcheck="false" placeholder="Search apps, windows, settings, or use pass ..." />
    </div>
    <section id="results" class="results" aria-live="polite"></section>
    <footer class="footer">
      <div id="status" class="status">Type to search. Arrow keys, Enter, click, or ⌥1-9.</div>
      <div>Esc closes</div>
    </footer>
  </main>
  <script>
    const state = {{
      query: "",
      items: [],
      selectedIndex: 0,
      hoverSuspended: false,
    }};

    const resultsEl = document.getElementById("results");
    const inputEl = document.getElementById("query");
    const statusEl = document.getElementById("status");

    const send = (payload) => window.ipc.postMessage(JSON.stringify(payload));

    const clampSelection = () => {{
      if (state.items.length === 0) {{
        state.selectedIndex = 0;
        return;
      }}
      state.selectedIndex = Math.max(0, Math.min(state.selectedIndex, state.items.length - 1));
    }};

    const moveSelection = (delta) => {{
      if (state.items.length === 0) {{
        return;
      }}
      state.selectedIndex = Math.max(0, Math.min(state.selectedIndex + delta, state.items.length - 1));
      state.hoverSuspended = true;
      render();
    }};

    const render = () => {{
      clampSelection();
      resultsEl.innerHTML = "";

      if (state.items.length === 0) {{
        const empty = document.createElement("div");
        empty.className = "item selected";
        empty.innerHTML = `
          <div class="badge">RUN</div>
          <div class="copy">
            <div class="title">No results yet</div>
            <div class="subtitle">Try an app name, a settings pane, or a plugin trigger like <code>pass</code>.</div>
          </div>
          <div class="accelerator">⌥Space</div>
        `;
        resultsEl.appendChild(empty);
        return;
      }}

      state.items.forEach((item, index) => {{
        const row = document.createElement("button");
        row.type = "button";
        row.className = "item" + (index === state.selectedIndex ? " selected" : "");
        const badgeMarkup = item.icon
          ? `<div class="badge has-icon"><img class="icon-image" src="${{escapeAttr(item.icon)}}" alt="" /></div>`
          : `<div class="badge">${{item.badge}}</div>`;
        row.innerHTML = `
          ${{badgeMarkup}}
          <div class="copy">
            <div class="title">${{escapeHtml(item.title)}}</div>
            <div class="subtitle">${{escapeHtml(item.subtitle)}}</div>
          </div>
          <div class="accelerator">${{item.accelerator ? escapeHtml(item.accelerator) : ""}}</div>
        `;
        row.addEventListener("mousemove", () => {{
          if (state.hoverSuspended || state.selectedIndex === index) {{
            return;
          }}
          state.selectedIndex = index;
          render();
        }});
        row.addEventListener("click", () => send({{ type: "activate", index }}));
        resultsEl.appendChild(row);
      }});

      const selected = resultsEl.children[state.selectedIndex];
      if (selected) {{
        selected.scrollIntoView({{ block: "nearest" }});
      }}
    }};

    const escapeHtml = (value) =>
      value
        .replaceAll("&", "&amp;")
        .replaceAll("<", "&lt;")
        .replaceAll(">", "&gt;");

    const escapeAttr = (value) => escapeHtml(value).replaceAll('"', "&quot;");

    window.__RUNX_RENDER = (payload) => {{
      state.query = payload.query;
      state.items = payload.items || [];
      if (typeof payload.query === "string" && inputEl.value !== payload.query) {{
        inputEl.value = payload.query;
      }}
      statusEl.textContent = payload.status?.message || "Type to search. Arrow keys, Enter, click, or ⌥1-9.";
      statusEl.className = "status" + (payload.status?.kind === "error" ? " error" : "");
      render();
    }};

    window.__RUNX_FOCUS = () => {{
      inputEl.focus();
      inputEl.select();
    }};

    inputEl.addEventListener("input", () => {{
      send({{ type: "query_changed", query: inputEl.value }});
    }});

    resultsEl.addEventListener("mousemove", () => {{
      state.hoverSuspended = false;
    }});

    inputEl.addEventListener("keydown", (event) => {{
      const ctrlDown = event.ctrlKey && !event.metaKey && !event.altKey;

      if (event.key === "ArrowDown" || (ctrlDown && event.key.toLowerCase() === "n")) {{
        event.preventDefault();
        moveSelection(1);
        return;
      }}

      if (event.key === "ArrowUp" || (ctrlDown && event.key.toLowerCase() === "p")) {{
        event.preventDefault();
        moveSelection(-1);
        return;
      }}

      if (event.key === "Enter") {{
        event.preventDefault();
        send({{ type: "activate", index: state.selectedIndex }});
        return;
      }}

      if (event.key === "Escape") {{
        event.preventDefault();
        send({{ type: "hide" }});
        return;
      }}

      if (event.altKey && /^Digit[1-9]$/.test(event.code)) {{
        const index = Number(event.code.slice("Digit".length)) - 1;
        if (index < state.items.length) {{
          event.preventDefault();
          send({{ type: "activate", index }});
        }}
      }}
    }});

    send({{ type: "ready" }});
  </script>
</body>
</html>"#,
        accent = theme.accent,
        background = theme.background,
        panel = theme.panel,
        text = theme.text,
        muted = theme.muted,
        font_family = theme.font_family,
    )
}
