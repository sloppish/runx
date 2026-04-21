(function (root) {
  function createState() {
    return {
      query: "",
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

  function moveSelection(state, delta) {
    if (state.items.length === 0) {
      return;
    }

    state.selectedIndex = Math.max(0, Math.min(state.selectedIndex + delta, state.items.length - 1));
  }

  function inputChanged(state, query) {
    state.query = query;
    state.selectedIndex = 0;
  }

  function applyRenderPayload(state, payload, environment) {
    const payloadQuery = typeof payload.query === "string" ? payload.query : "";
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
  const inputEl = document.getElementById("query");
  const send = (payload) => root.ipc.postMessage(JSON.stringify(payload));

  function syncSelection(ensureVisible = true) {
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
      row.addEventListener("click", () => send({ type: "activate", index }));
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
    inputEl.focus();
    inputEl.select();
  };

  inputEl.addEventListener("input", () => {
    inputChanged(state, inputEl.value);
    syncSelection(false);
    send({ type: "query_changed", query: inputEl.value });
  });

  inputEl.addEventListener("keydown", (event) => {
    const ctrlDown = event.ctrlKey && !event.metaKey && !event.altKey;

    if (event.key === "ArrowDown" || (ctrlDown && event.key.toLowerCase() === "n")) {
      event.preventDefault();
      moveSelection(state, 1);
      render();
      return;
    }

    if (event.key === "ArrowUp" || (ctrlDown && event.key.toLowerCase() === "p")) {
      event.preventDefault();
      moveSelection(state, -1);
      render();
      return;
    }

    if (event.key === "Enter") {
      event.preventDefault();
      send({ type: "activate", index: state.selectedIndex });
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
        send({ type: "activate", index });
      }
    }
  });

  send({ type: "ready" });
  root.RunxUi = api;
})(typeof window !== "undefined" ? window : globalThis);
