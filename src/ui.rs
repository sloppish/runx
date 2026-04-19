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
    }}

    * {{
      box-sizing: border-box;
    }}

    html, body {{
      width: 100%;
      height: 100%;
      margin: 0;
      overflow: hidden;
      background:
        radial-gradient(circle at top, rgba(255, 255, 255, 0.68), transparent 42%),
        linear-gradient(180deg, rgba(255, 255, 255, 0.78) 0%, rgba(250, 243, 233, 0.96) 100%);
      color: var(--text);
      font-family: var(--font);
      -webkit-font-smoothing: antialiased;
      user-select: none;
    }}

    body {{
      padding: 20px;
    }}

    .shell {{
      height: 100%;
      display: flex;
      flex-direction: column;
      gap: 14px;
      padding: 18px;
      border-radius: 24px;
      background:
        linear-gradient(180deg, rgba(255, 255, 255, 0.96), rgba(252, 246, 238, 0.98));
      box-shadow:
        0 32px 80px rgba(81, 55, 31, 0.16),
        inset 0 1px 0 rgba(255, 255, 255, 0.72);
      border: 1px solid rgba(140, 102, 67, 0.16);
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
      color: color-mix(in srgb, var(--text) 88%, white);
      font-weight: 700;
    }}

    .input-wrap {{
      position: relative;
      border-radius: 18px;
      padding: 14px 18px;
      background: linear-gradient(180deg, rgba(255,255,255,0.92), rgba(248,240,230,0.9));
      border: 1px solid rgba(140, 102, 67, 0.14);
      box-shadow: inset 0 1px 0 rgba(255,255,255,0.72);
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
    }}

    input::placeholder {{
      color: color-mix(in srgb, var(--muted) 78%, white);
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
      background: rgba(134, 98, 66, 0.18);
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
      background: rgba(255,255,255,0.45);
      cursor: pointer;
      transition: transform 120ms ease, background 120ms ease, box-shadow 120ms ease;
      text-align: left;
      color: inherit;
      font: inherit;
    }}

    .item:hover {{
      transform: translateY(-1px);
      background: rgba(255,255,255,0.86);
    }}

    .item.selected {{
      background: linear-gradient(135deg, rgba(199, 123, 73, 0.15), rgba(255,255,255,0.94));
      box-shadow:
        0 18px 44px rgba(139, 87, 46, 0.14),
        inset 0 0 0 1px rgba(199, 123, 73, 0.22);
    }}

    .badge {{
      min-width: 46px;
      height: 46px;
      padding: 0 12px;
      border-radius: 14px;
      background: linear-gradient(180deg, color-mix(in srgb, var(--accent) 18%, white), rgba(255,255,255,0.95));
      border: 1px solid rgba(199, 123, 73, 0.18);
      display: grid;
      place-items: center;
      font-size: 11px;
      font-weight: 700;
      letter-spacing: 0.18em;
      color: color-mix(in srgb, var(--accent) 72%, black);
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
      color: color-mix(in srgb, var(--muted) 80%, white);
      font-size: 12px;
      font-weight: 600;
      padding: 6px 9px;
      border-radius: 999px;
      background: rgba(255,255,255,0.68);
      border: 1px solid rgba(140, 102, 67, 0.12);
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
      color: #9c4420;
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
        row.innerHTML = `
          <div class="badge">${{item.badge}}</div>
          <div class="copy">
            <div class="title">${{escapeHtml(item.title)}}</div>
            <div class="subtitle">${{escapeHtml(item.subtitle)}}</div>
          </div>
          <div class="accelerator">${{item.accelerator ? escapeHtml(item.accelerator) : ""}}</div>
        `;
        row.addEventListener("mouseenter", () => {{
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

    inputEl.addEventListener("keydown", (event) => {{
      if (event.key === "ArrowDown") {{
        event.preventDefault();
        if (state.items.length > 0) {{
          state.selectedIndex = Math.min(state.selectedIndex + 1, state.items.length - 1);
          render();
        }}
        return;
      }}

      if (event.key === "ArrowUp") {{
        event.preventDefault();
        if (state.items.length > 0) {{
          state.selectedIndex = Math.max(state.selectedIndex - 1, 0);
          render();
        }}
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

      if (event.altKey && /^[1-9]$/.test(event.key)) {{
        const index = Number(event.key) - 1;
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
