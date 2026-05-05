(function (root) {
  function createState() {
    return {
      query: "",
      configError: null,
      items: [],
      selectedIndex: 0,
    };
  }

  function normalizeVisibleRows(value) {
    const parsed = Number(value);
    if (!Number.isFinite(parsed)) {
      return 1;
    }

    return Math.max(1, Math.round(parsed));
  }

  function clampSelection(state) {
    if (state.items.length === 0) {
      state.selectedIndex = 0;
      return;
    }

    state.selectedIndex = Math.max(0, Math.min(state.selectedIndex, state.items.length - 1));
  }

  function moveSelection(state, delta, cycle = false) {
    if (state.items.length === 0) {
      return;
    }

    if (!cycle) {
      state.selectedIndex = Math.max(0, Math.min(state.selectedIndex + delta, state.items.length - 1));
      return;
    }

    const count = state.items.length;
    state.selectedIndex = ((state.selectedIndex + delta) % count + count) % count;
  }

  function inputChanged(state, query) {
    state.query = query;
    state.selectedIndex = 0;
  }

  function isCtrlNextShortcut(event) {
    return event.key === "ArrowDown" || (event.ctrlKey && !event.metaKey && !event.altKey && event.code === "KeyN");
  }

  function isCtrlPreviousShortcut(event) {
    return event.key === "ArrowUp" || (event.ctrlKey && !event.metaKey && !event.altKey && event.code === "KeyP");
  }

  function isCopyShortcut(event) {
    return event.code === "KeyC" && event.metaKey && !event.ctrlKey && !event.altKey && !event.shiftKey;
  }

  function isPasteShortcut(event) {
    return event.code === "KeyV" && event.metaKey && !event.ctrlKey && !event.altKey && !event.shiftKey;
  }

  function selectedInputText(input) {
    const start = input.selectionStart ?? input.value.length;
    const end = input.selectionEnd ?? input.value.length;
    if (start === end) {
      return "";
    }
    return input.value.slice(Math.min(start, end), Math.max(start, end));
  }

  function replaceInputSelection(input, text) {
    const start = input.selectionStart ?? input.value.length;
    const end = input.selectionEnd ?? input.value.length;
    const head = input.value.slice(0, Math.min(start, end));
    const tail = input.value.slice(Math.max(start, end));
    const next = head + text + tail;
    const cursor = head.length + text.length;
    input.value = next;
    if (input.setSelectionRange) {
      input.setSelectionRange(cursor, cursor);
    }
    return next;
  }

  function matchesShortcut(event, shortcut) {
    if (!shortcut) {
      return false;
    }

    const keyMatches = !shortcut.key || event.key === shortcut.key;
    const codeMatches = !shortcut.code || event.code === shortcut.code;
    return (
      keyMatches &&
      codeMatches &&
      !!event.altKey === !!shortcut.alt &&
      !!event.ctrlKey === !!shortcut.ctrl &&
      !!event.metaKey === !!shortcut.meta &&
      !!event.shiftKey === !!shortcut.shift
    );
  }

  function applyRenderPayload(state, payload, environment) {
    const payloadQuery = typeof payload.query === "string" ? payload.query : "";
    state.configError = typeof payload.config_error === "string" ? payload.config_error : null;
    const shouldSyncInput =
      payloadQuery === "" ||
      environment.inputValue === payloadQuery ||
      !environment.inputFocused;
    const queryChanged = state.query !== payloadQuery;

    if (shouldSyncInput) {
      state.query = payloadQuery;
    }

    state.items = payload.items || [];
    if (queryChanged && shouldSyncInput) {
      state.selectedIndex = 0;
    }

    return {
      shouldSyncInput,
      inputValue: payloadQuery,
    };
  }

  function escapeHtml(value) {
    return value
      .replaceAll("&", "&amp;")
      .replaceAll("<", "&lt;")
      .replaceAll(">", "&gt;");
  }

  function escapeAttr(value) {
    return escapeHtml(value).replaceAll('"', "&quot;");
  }

  const api = {
    applyRenderPayload,
    clampSelection,
    createState,
    escapeAttr,
    escapeHtml,
    inputChanged,
    isCopyShortcut,
    isCtrlNextShortcut,
    isCtrlPreviousShortcut,
    isPasteShortcut,
    matchesShortcut,
    moveSelection,
    normalizeVisibleRows,
    replaceInputSelection,
    selectedInputText,
  };

  if (typeof module !== "undefined" && module.exports) {
    module.exports = api;
  }

  if (!root || !root.document) {
    root.RunxUi = api;
    return;
  }

  const state = createState();
  const shellEl = document.querySelector("main");
  const resultsEl = document.getElementById("results");
  const configErrorEl = document.getElementById("config-error");
  const inputEl = document.getElementById("query");
  const inputWrapEl = document.querySelector(".input-wrap");
  const send = (payload) => root.ipc.postMessage(JSON.stringify(payload));
  let pendingPreferredHeightTimer = null;
  let lastReportedPreferredHeight = null;
  let lastReportedLayoutVersion = null;

  function cycleSelectionEnabled() {
    return !!root.__RUNX_CYCLE_SELECTION__;
  }

  function activateAllWindowsShortcut() {
    return root.__RUNX_ACTIVATE_ALL_WINDOWS_SHORTCUT__ || null;
  }

  function focusWindowShortcut() {
    return root.__RUNX_FOCUS_WINDOW_SHORTCUT__ || null;
  }

  function inConfigErrorMode() {
    return !!state.configError && state.query === "";
  }

  function visibleRows() {
    return normalizeVisibleRows(root.__RUNX_VISIBLE_ROWS__);
  }

  function layoutVersion() {
    const parsed = Number(root.__RUNX_LAYOUT_VERSION__);
    if (!Number.isFinite(parsed) || parsed < 0) {
      return 0;
    }

    return Math.floor(parsed);
  }

  function measurementWidth() {
    return Math.max(
      Math.ceil(shellEl.getBoundingClientRect().width),
      document.documentElement.clientWidth,
      720,
    );
  }

  function createMeasurementItem() {
    const row = document.createElement("button");
    row.type = "button";
    row.className = "item";
    row.innerHTML = `
      <div class="badge">AP</div>
      <div class="copy">
        <div class="title">Measure Row</div>
        <div class="subtitle">Preferred launcher height</div>
      </div>
      <div class="accelerator">Return</div>
    `;
    return row;
  }

  function measurePreferredHeight() {
    const host = document.createElement("div");
    host.style.position = "fixed";
    host.style.left = "-10000px";
    host.style.top = "0";
    host.style.visibility = "hidden";
    host.style.pointerEvents = "none";
    host.style.padding = root.getComputedStyle(document.body).padding;

    const shell = document.createElement("main");
    shell.className = shellEl.className;
    shell.style.height = "auto";
    shell.style.width = `${measurementWidth()}px`;
    shell.style.overflow = "visible";

    const label = document.createElement("div");
    label.className = "label";
    label.innerHTML = "<strong>Runx</strong>";

    const inputWrap = document.createElement("div");
    inputWrap.className = "input-wrap";
    inputWrap.innerHTML = '<input type="text" autocomplete="off" spellcheck="false" placeholder="" value="" />';

    const results = document.createElement("section");
    results.className = "results";
    results.style.flex = "none";
    results.style.minHeight = "auto";
    results.style.overflow = "visible";

    for (let index = 0; index < visibleRows(); index += 1) {
      results.appendChild(createMeasurementItem());
    }

    shell.appendChild(label);
    shell.appendChild(inputWrap);
    shell.appendChild(results);
    host.appendChild(shell);
    document.body.appendChild(host);

    try {
      return Math.ceil(host.getBoundingClientRect().height);
    } finally {
      host.remove();
    }
  }

  function reportPreferredHeight() {
    const height = measurePreferredHeight();
    const version = layoutVersion();
    if (!Number.isFinite(height) || height <= 0) {
      return;
    }

    if (lastReportedPreferredHeight === height && lastReportedLayoutVersion === version) {
      return;
    }

    lastReportedPreferredHeight = height;
    lastReportedLayoutVersion = version;
    send({ type: "preferred_height", height, layout_version: version });
  }

  function queuePreferredHeightReport() {
    if (pendingPreferredHeightTimer !== null) {
      root.clearTimeout(pendingPreferredHeightTimer);
    }

    pendingPreferredHeightTimer = root.setTimeout(() => {
      pendingPreferredHeightTimer = null;
      reportPreferredHeight();
    }, 0);
  }

  function syncSelection(ensureVisible = true) {
    if (inConfigErrorMode()) {
      return;
    }

    Array.from(resultsEl.children).forEach((child, index) => {
      child.classList.toggle("selected", index === state.selectedIndex);
    });

    if (!ensureVisible) {
      return;
    }

    const selected = resultsEl.children[state.selectedIndex];
    if (selected) {
      selected.scrollIntoView({ block: "nearest" });
    }
  }

  function render() {
    clampSelection(state);

    if (inConfigErrorMode()) {
      inputWrapEl.classList.add("hidden");
      resultsEl.classList.add("hidden");
      configErrorEl.classList.remove("hidden");
      configErrorEl.innerHTML = `
        <div class="config-error-title">Config reload failed</div>
        <div class="config-error-copy">${escapeHtml(state.configError)}</div>
      `;
      return;
    }

    inputWrapEl.classList.remove("hidden");
    resultsEl.classList.remove("hidden");
    configErrorEl.classList.add("hidden");
    resultsEl.innerHTML = "";

    for (const [index, item] of state.items.entries()) {
      const row = document.createElement("button");
      row.type = "button";
      row.className = "item" + (item.compact ? " compact" : "") + (index === state.selectedIndex ? " selected" : "");
      const badgeMarkup = item.compact
        ? ""
        : item.icon
        ? `<div class="badge has-icon"><img class="icon-image" src="${escapeAttr(item.icon)}" alt="" /></div>`
        : `<div class="badge">${item.badge}</div>`;
      const subtitleText = item.subtitle || "";
      const subtitleMarkup = item.compact
        ? ""
        : `<div class="subtitle" title="${escapeAttr(subtitleText)}">${escapeHtml(subtitleText)}</div>`;
      row.innerHTML = `
        ${badgeMarkup}
        <div class="copy">
          <div class="title">${escapeHtml(item.title)}</div>
          ${subtitleMarkup}
        </div>
        <div class="accelerator">${item.accelerator ? escapeHtml(item.accelerator) : ""}</div>
      `;
      row.addEventListener("click", () => send({ type: "activate", index, all_windows: false }));
      resultsEl.appendChild(row);
    }

    syncSelection();
  }

  root.__RUNX_RENDER = (payload) => {
    const update = applyRenderPayload(state, payload, {
      inputValue: inputEl.value,
      inputFocused: document.activeElement === inputEl,
    });

    if (update.shouldSyncInput && inputEl.value !== update.inputValue) {
      inputEl.value = update.inputValue;
    }

    render();
  };

  root.__RUNX_FOCUS = () => {
    if (inConfigErrorMode()) {
      return;
    }
    inputEl.focus();
    inputEl.select();
  };

  root.__RUNX_PASTE_TEXT = (text) => {
    if (inConfigErrorMode()) {
      return;
    }
    inputEl.focus();
    const query = replaceInputSelection(inputEl, text || "");
    inputChanged(state, query);
    syncSelection(false);
    send({ type: "query_changed", query });
  };

  root.__RUNX_REQUEST_PREFERRED_HEIGHT = () => {
    queuePreferredHeightReport();
  };

  inputEl.addEventListener("input", () => {
    inputChanged(state, inputEl.value);
    syncSelection(false);
    send({ type: "query_changed", query: inputEl.value });
  });

  inputEl.addEventListener("keydown", (event) => {
    if (isCopyShortcut(event)) {
      const text = selectedInputText(inputEl);
      if (text) {
        event.preventDefault();
        send({ type: "copy_text", text });
      }
      return;
    }

    if (isPasteShortcut(event)) {
      event.preventDefault();
      send({ type: "paste_text" });
      return;
    }

    if (event.code === "KeyA" && event.metaKey && !event.altKey && !event.shiftKey && !event.ctrlKey) {
      event.preventDefault();
      inputEl.select();
      return;
    }

    if (isCtrlNextShortcut(event)) {
      event.preventDefault();
      moveSelection(state, 1, cycleSelectionEnabled());
      syncSelection();
      return;
    }

    if (isCtrlPreviousShortcut(event)) {
      event.preventDefault();
      moveSelection(state, -1, cycleSelectionEnabled());
      syncSelection();
      return;
    }

    if (matchesShortcut(event, activateAllWindowsShortcut())) {
      event.preventDefault();
      send({ type: "activate", index: state.selectedIndex, all_windows: true });
      return;
    }

    if (matchesShortcut(event, focusWindowShortcut())) {
      event.preventDefault();
      send({ type: "activate", index: state.selectedIndex, all_windows: false });
      return;
    }

    if (event.key === "Escape") {
      event.preventDefault();
      send({ type: "hide" });
      return;
    }

    if (event.altKey && /^Digit[1-9]$/.test(event.code)) {
      const index = Number(event.code.slice("Digit".length)) - 1;
      if (index < state.items.length) {
        event.preventDefault();
        send({ type: "activate", index, all_windows: false });
      }
    }
  });

  document.addEventListener("keydown", (event) => {
    if (!inConfigErrorMode()) {
      return;
    }

    if (event.key === "Escape") {
      event.preventDefault();
      send({ type: "hide" });
      return;
    }

    if (isCopyShortcut(event)) {
      const selection = root.getSelection ? root.getSelection() : null;
      const text = selection ? selection.toString() : "";
      if (text) {
        event.preventDefault();
        send({ type: "copy_text", text });
      }
      return;
    }

    if (event.metaKey || event.ctrlKey || event.altKey || event.key.length !== 1) {
      return;
    }

    event.preventDefault();
    inputEl.value = event.key;
    inputChanged(state, inputEl.value);
    render();
    inputEl.focus();
    send({ type: "query_changed", query: inputEl.value });
  });

  send({ type: "ready" });
  queuePreferredHeightReport();
  root.RunxUi = api;
})(typeof window !== "undefined" ? window : globalThis);
