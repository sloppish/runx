(function (root) {
  function createState() {
    return {
      query: "",
      configError: null,
      items: [],
      selectedIndex: 0,
    };
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
    isCtrlNextShortcut,
    isCtrlPreviousShortcut,
    matchesShortcut,
    moveSelection,
  };

  if (typeof module !== "undefined" && module.exports) {
    module.exports = api;
  }

  if (!root || !root.document) {
    root.RunxUi = api;
    return;
  }

  const state = createState();
  const resultsEl = document.getElementById("results");
  const configErrorEl = document.getElementById("config-error");
  const inputEl = document.getElementById("query");
  const inputWrapEl = document.querySelector(".input-wrap");
  const send = (payload) => root.ipc.postMessage(JSON.stringify(payload));

  function cycleSelectionEnabled() {
    return !!root.__RUNX_CYCLE_SELECTION__;
  }

  function activateAllWindowsShortcut() {
    return root.__RUNX_ACTIVATE_ALL_WINDOWS_SHORTCUT__ || null;
  }

  function inConfigErrorMode() {
    return !!state.configError && state.query === "";
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
      const subtitleMarkup = item.compact
        ? ""
        : `<div class="subtitle">${escapeHtml(item.subtitle)}</div>`;
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

  inputEl.addEventListener("input", () => {
    inputChanged(state, inputEl.value);
    syncSelection(false);
    send({ type: "query_changed", query: inputEl.value });
  });

  inputEl.addEventListener("keydown", (event) => {
    if (isCtrlNextShortcut(event)) {
      event.preventDefault();
      moveSelection(state, 1, cycleSelectionEnabled());
      render();
      return;
    }

    if (isCtrlPreviousShortcut(event)) {
      event.preventDefault();
      moveSelection(state, -1, cycleSelectionEnabled());
      render();
      return;
    }

    if (matchesShortcut(event, activateAllWindowsShortcut())) {
      event.preventDefault();
      send({ type: "activate", index: state.selectedIndex, all_windows: true });
      return;
    }

    if (event.key === "Enter") {
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
  root.RunxUi = api;
})(typeof window !== "undefined" ? window : globalThis);
