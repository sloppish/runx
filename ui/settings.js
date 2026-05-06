(function (root) {
  const send = (payload) => root.ipc.postMessage(JSON.stringify(payload));
  document.addEventListener("contextmenu", (event) => event.preventDefault());
  document.addEventListener("click", (e) => {
    const link = e.target.closest("[data-url]");
    if (link) {
      e.preventDefault();
      send({ type: "open_url", url: link.dataset.url });
    }
  });
  const panes = Array.from(document.querySelectorAll("[data-pane-panel]"));
  const navItems = Array.from(document.querySelectorAll("[data-pane]"));
  const statusEl = document.getElementById("status");
  const saveButton = document.getElementById("save");
  const formEl = document.getElementById("settings-form");
  const rawEl = document.getElementById("raw-toml");
  const providerListEl = document.getElementById("provider-list");
  const providerOrderEl = document.getElementById("provider-order-list");
  const displayOverridesEl = document.getElementById("display-overrides");
  const providerBoostsEl = document.getElementById("provider-score-boosts");
  const scoreRulesEl = document.getElementById("score-rules");
  const colorschemesEl = document.getElementById("custom-colorschemes");
  const configPathEl = document.getElementById("config-path");
  let state = root.__RUNX_INITIAL_SETTINGS__;
  let activePane = "general";
  let cleanSnapshots = { raw: null, structured: null };
  let isRendering = false;
  let isSaving = false;

  function field(path) {
    return document.querySelector(`[data-field="${path}"]`);
  }

  function setStatus(message, isError = false) {
    statusEl.textContent = message || "";
    statusEl.classList.toggle("visible", !!message);
    statusEl.classList.toggle("error", !!isError);
    if (!isRendering && message !== "Saving...") {
      isSaving = false;
      updateSaveButtonState();
    }
  }

  function setPane(name) {
    activePane = name;
    for (const item of navItems) {
      item.classList.toggle("active", item.dataset.pane === name);
    }
    formEl.style.display = name === "raw" ? "none" : "";
    for (const pane of panes) {
      pane.classList.toggle("active", pane.dataset.panePanel === name);
    }
    updateDirtyState();
  }

  function activeEditorKey() {
    return activePane === "raw" ? "raw" : "structured";
  }

  function editorSnapshot(key = activeEditorKey()) {
    if (key === "raw") {
      return JSON.stringify(rawEl.value);
    }
    if (!state?.draft) {
      return null;
    }
    return JSON.stringify(collectDraft());
  }

  function captureCleanSnapshots() {
    cleanSnapshots = {
      raw: editorSnapshot("raw"),
      structured: state?.draft ? editorSnapshot("structured") : null,
    };
  }

  function activeEditorCanSave() {
    return activeEditorKey() === "raw" || !!state?.draft;
  }

  function activeEditorIsDirty() {
    const key = activeEditorKey();
    return cleanSnapshots[key] != null && editorSnapshot(key) !== cleanSnapshots[key];
  }

  function updateSaveButtonState() {
    saveButton.disabled = isSaving || !activeEditorCanSave() || !activeEditorIsDirty();
  }

  function updateDirtyState() {
    if (isRendering) {
      return;
    }
    const dirty = activeEditorIsDirty();
    updateSaveButtonState();
    if (isSaving) {
      return;
    }
    if (dirty) {
      setStatus("Unsaved changes");
    } else if (statusEl.textContent === "Unsaved changes" || statusEl.textContent === "Saving...") {
      setStatus("");
    }
  }

  function pathValue(object, path) {
    return path.split(".").reduce((value, part) => (value == null ? value : value[part]), object);
  }

  function setField(path, value) {
    const input = field(path);
    if (!input) {
      return;
    }
    if (input.type === "checkbox") {
      input.checked = !!value;
    } else if (Array.isArray(value)) {
      input.value = input.tagName === "TEXTAREA" ? value.join("\n") : value.join(", ");
    } else {
      input.value = value ?? "";
    }
  }

  function bool(path) {
    return !!field(path).checked;
  }

  function text(path) {
    return field(path).value.trim();
  }

  function rawText(path) {
    return field(path).value;
  }

  function list(path) {
    return text(path)
      .split(",")
      .map((item) => item.trim())
      .filter(Boolean);
  }

  function lineList(path) {
    return rawText(path)
      .split(/\r?\n/)
      .map((item) => item.trim())
      .filter(Boolean);
  }

  function float(path) {
    return Number(field(path).value);
  }

  function int(path) {
    return Math.trunc(Number(field(path).value));
  }

  function optionalNumber(input, integer = false) {
    const value = input.value.trim();
    if (!value) {
      return null;
    }
    const parsed = Number(value);
    return integer ? Math.trunc(parsed) : parsed;
  }

  function optionalFloat(path) {
    return optionalNumber(field(path));
  }

  function optionalBool(select) {
    if (select.value === "") {
      return null;
    }
    return select.value === "true";
  }

  function normalizeNumberInput(input) {
    if (input.type !== "number") {
      return;
    }
    input.type = "text";
    input.inputMode = "decimal";
  }



  let recordingShortcut = null;

  function modifierParts(event) {
    const parts = [];
    if (event.ctrlKey) {
      parts.push("Ctrl");
    }
    if (event.altKey) {
      parts.push("Option");
    }
    if (event.shiftKey) {
      parts.push("Shift");
    }
    if (event.metaKey) {
      parts.push("Cmd");
    }
    return parts;
  }

  function modifierDisplay(value) {
    switch (value.toLowerCase()) {
      case "alt":
      case "option":
        return "Option";
      case "control":
      case "ctrl":
        return "Ctrl";
      case "command":
      case "cmd":
      case "super":
      case "meta":
        return "Cmd";
      case "shift":
        return "Shift";
      default:
        return value;
    }
  }

  function displayKeyToken(token) {
    if (!token) {
      return "";
    }
    if (/^Key[A-Z]$/.test(token)) {
      return token.slice(3);
    }
    if (/^Digit[0-9]$/.test(token)) {
      return token.slice(5);
    }
    if (token === "ArrowUp") {
      return "Up";
    }
    if (token === "ArrowDown") {
      return "Down";
    }
    if (token === "ArrowLeft") {
      return "Left";
    }
    if (token === "ArrowRight") {
      return "Right";
    }
    return token;
  }

  function displayShortcutText(value, emptyLabel = "Disabled") {
    const textValue = (value || "").trim();
    if (!textValue) {
      return emptyLabel;
    }
    if (/^(none|disabled|off)$/i.test(textValue)) {
      return "Disabled";
    }
    return textValue
      .split("+")
      .map((part) => part.trim())
      .filter(Boolean)
      .map((part) => modifierDisplay(displayKeyToken(part)))
      .join("+");
  }

  function isModifierOnlyKey(event) {
    return ["Alt", "Control", "Meta", "Shift"].includes(event.key);
  }

  function shortcutKeyFromEvent(event) {
    const code = event.code || "";
    if (/^Key[A-Z]$/.test(code) || /^Digit[0-9]$/.test(code) || /^Numpad[0-9]$/.test(code)) {
      return code;
    }
    if ([
      "Space",
      "Enter",
      "NumpadEnter",
      "Escape",
      "Tab",
      "Backspace",
      "Delete",
      "ArrowUp",
      "ArrowDown",
      "ArrowLeft",
      "ArrowRight",
    ].includes(code)) {
      return code;
    }
    return null;
  }

  function syncShortcutRecorders() {
    for (const button of formEl.querySelectorAll("[data-shortcut-recorder]")) {
      if (button === recordingShortcut) {
        button.textContent = "Press shortcut...";
        button.classList.add("recording");
        button.classList.remove("empty");
        continue;
      }

      const target = button.dataset.shortcutRecorder;
      const textValue = displayShortcutText(field(target).value, "Record shortcut");
      button.textContent = textValue;
      button.classList.toggle("empty", textValue === "Disabled" || textValue === "Record shortcut");
      button.classList.remove("recording");
    }
  }

  function finishShortcutRecording() {
    recordingShortcut = null;
    syncShortcutRecorders();
  }

  function recordShortcut(button, event) {
    if (event.key === "Escape" && modifierParts(event).length === 0) {
      finishShortcutRecording();
      return;
    }
    if (isModifierOnlyKey(event)) {
      return;
    }

    const target = button.dataset.shortcutRecorder;
    const key = shortcutKeyFromEvent(event);
    if (!key) {
      setStatus("Unsupported shortcut key.", true);
      finishShortcutRecording();
      return;
    }
    field(target).value = [...modifierParts(event), key].join("+");

    finishShortcutRecording();
    updateDirtyState();
  }

  function validateStructuredForm() {
    const resultLimit = field("ranking.result_limit");
    if (resultLimit && !resultLimit.disabled && Number(resultLimit.value) < 1) {
      return "Result limit must be at least 1.";
    }

    const colorschemeNames = new Set();
    for (const input of colorschemesEl.querySelectorAll("[data-colorscheme-name]")) {
      const name = input.value.trim();
      if (colorschemeNames.has(name)) {
        return `Custom colorscheme ${name} is duplicated.`;
      }
      colorschemeNames.add(name);
    }

    const boostProviders = new Set();
    for (const select of providerBoostsEl.querySelectorAll("[data-boost-provider]")) {
      if (boostProviders.has(select.value)) {
        return `Provider score boost for ${select.value} is duplicated.`;
      }
      boostProviders.add(select.value);
    }

    const displayOverrides = new Set();
    for (const select of displayOverridesEl.querySelectorAll("[data-display-target]")) {
      if (select.value === "manual") {
        continue;
      }
      if (displayOverrides.has(select.value)) {
        return "Display override targets must not be duplicated.";
      }
      displayOverrides.add(select.value);
    }

    return null;
  }

  function option(value, label = value) {
    const node = document.createElement("option");
    node.value = value;
    node.textContent = label;
    return node;
  }

  function actionButton(label, action) {
    const button = document.createElement("button");
    button.type = "button";
    button.className = "button compact";
    button.textContent = label;
    button.addEventListener("click", action);
    return button;
  }

  function card(title, onRemove) {
    const wrapper = document.createElement("div");
    wrapper.className = "collection-item";
    const header = document.createElement("div");
    header.className = "item-header";
    const heading = document.createElement("h3");
    heading.textContent = title;
    header.append(
      heading,
      actionButton("Delete", () => {
        onRemove(wrapper);
        updateDirtyState();
      }),
    );
    wrapper.append(header);
    return wrapper;
  }

  function labeledInput(label, type, className) {
    const wrapper = document.createElement("label");
    const span = document.createElement("span");
    const input = document.createElement("input");
    span.textContent = label;
    input.type = type;
    normalizeNumberInput(input);
    if (className) {
      input.className = className;
    }
    wrapper.append(span, input);
    return [wrapper, input];
  }

  function checkedProviders() {
    return Array.from(providerListEl.querySelectorAll("input"))
      .filter((input) => !input.checked)
      .map((input) => input.value);
  }

  function renderProviders(payload) {
    providerListEl.replaceChildren();
    const disabled = new Set(payload.draft?.providers?.disabled || []);
    for (const provider of payload.known_providers || []) {
      const label = document.createElement("label");
      label.className = "toggle";
      const input = document.createElement("input");
      input.type = "checkbox";
      input.value = provider;
      input.checked = !disabled.has(provider);
      const span = document.createElement("span");
      span.textContent = provider;
      label.append(input, span);
      providerListEl.append(label);
    }
  }

  function renderProviderOrder(order) {
    providerOrderEl.replaceChildren();
    for (const name of order) {
      const row = document.createElement("div");
      row.className = "order-item";
      row.dataset.provider = name;
      const label = document.createElement("span");
      label.textContent = name;
      const up = document.createElement("button");
      up.type = "button";
      up.className = "order-btn";
      up.textContent = "↑";
      up.addEventListener("click", () => {
        const prev = row.previousElementSibling;
        if (prev) {
          providerOrderEl.insertBefore(row, prev);
          updateDirtyState();
        }
      });
      const down = document.createElement("button");
      down.type = "button";
      down.className = "order-btn";
      down.textContent = "↓";
      down.addEventListener("click", () => {
        const next = row.nextElementSibling;
        if (next) {
          providerOrderEl.insertBefore(next, row);
          updateDirtyState();
        }
      });
      row.append(label, up, down);
      providerOrderEl.append(row);
    }
  }

  function collectProviderOrder() {
    return Array.from(providerOrderEl.querySelectorAll(".order-item")).map(
      (el) => el.dataset.provider
    );
  }

  function syncColorschemeOptions() {
    const select = field("ui.colorscheme");
    const previous = select.value || state.draft?.ui?.colorscheme || "system";
    const customNames = collectColorschemes().map((scheme) => scheme.name).filter(Boolean);
    const names = ["system", ...(state.builtin_colorschemes || []), ...customNames];
    select.replaceChildren();
    for (const name of names) {
      select.append(option(name));
    }
    select.value = names.includes(previous) ? previous : "system";
  }

  function renderColorschemes(payload) {
    const select = field("ui.colorscheme");
    select.replaceChildren();
    for (const name of payload.colorschemes || []) {
      select.append(option(name));
    }
  }

  function renderDraft(draft) {
    for (const input of formEl.querySelectorAll("[data-field]")) {
      setField(input.dataset.field, pathValue(draft, input.dataset.field));
    }
    syncShortcutRecorders();
  }

  function renderDisplayOverrides(overrides = []) {
    displayOverridesEl.replaceChildren();
    for (const entry of overrides) {
      addDisplayOverride(entry);
    }
  }

  function buildDisplayTargetBlock(selectedDisplay, disconnectedEntry) {
    const target = document.createElement("div");
    target.className = "display-target";
    const targetLabel = document.createElement("label");
    const targetText = document.createElement("span");
    const targetSelect = document.createElement("select");
    const targetSummary = document.createElement("div");
    targetText.textContent = "Display";
    targetSelect.dataset.displayTarget = "true";
    targetSummary.className = "display-identity-summary";
    for (const display of state.displays || []) {
      targetSelect.append(option(display.key, display.label));
    }
    if (disconnectedEntry) {
      const key = displayKeyFor(disconnectedEntry);
      const opt = option(key, disconnectedDisplayLabel(disconnectedEntry));
      opt.disabled = true;
      targetSelect.append(opt);
      targetSelect.append(option("manual", "Manual identity"));
      targetSelect.value = key;
    } else {
      targetSelect.append(option("manual", "Manual identity"));
      targetSelect.value = selectedDisplay?.key || "manual";
    }
    targetLabel.append(targetText, targetSelect);
    target.append(targetLabel, targetSummary);
    return { target, targetSelect, targetSummary };
  }

  function disconnectedDisplayLabel(entry) {
    const parts = [];
    if (entry.built_in != null) {
      parts.push(entry.built_in ? "Built-in" : "External");
    }
    if (entry.vendor != null) {
      parts.push(`vendor ${entry.vendor}`);
    }
    if (entry.model != null) {
      parts.push(`model ${entry.model}`);
    }
    if (entry.serial != null) {
      parts.push(`serial ${entry.serial}`);
    }
    const label = parts.length > 0 ? parts.join(" · ") : "Unknown display";
    return `${label} (disconnected)`;
  }

  function buildIdentityBlock(entry) {
    const identity = document.createElement("div");
    identity.className = "manual-identity grid four";

    const builtInLabel = document.createElement("label");
    const builtInText = document.createElement("span");
    const builtIn = document.createElement("select");
    builtIn.dataset.overrideField = "built_in";
    builtInText.textContent = "Built-in";
    builtIn.append(option("", "Any"), option("true", "Built-in"), option("false", "External"));
    builtIn.value = entry.built_in == null ? "" : String(entry.built_in);
    builtInLabel.append(builtInText, builtIn);
    identity.append(builtInLabel);

    for (const [key, label] of [
      ["vendor", "Vendor"],
      ["model", "Model"],
      ["serial", "Serial"],
    ]) {
      const [labelNode, input] = labeledInput(label, "text");
      input.dataset.overrideField = key;
      input.dataset.optional = "true";
      input.inputMode = "numeric";
      input.value = entry[key] ?? "";
      identity.append(labelNode);
    }
    return { identity, builtIn };
  }

  function buildDimensionsGrid(entry) {
    const grid = document.createElement("div");
    grid.className = "grid four";

    for (const [key, label, integer] of [
      ["width_fraction", "Width fraction", false],
      ["visible_rows", "Visible rows", true],
      ["min_width", "Min width", false],
      ["max_width", "Max width", false],
      ["min_height", "Min height", false],
      ["max_height", "Max height", false],
      ["scale", "Scale", false],
    ]) {
      const [labelNode, input] = labeledInput(label, "number");
      input.dataset.overrideField = key;
      input.dataset.optional = "true";
      input.step = integer ? "1" : "0.01";
      input.value = entry[key] ?? "";
      grid.append(labelNode);
    }
    return grid;
  }

  function addDisplayOverride(entry = {}, preferredDisplayKey = null, reveal = false) {
    if (Object.keys(entry).length === 0 && (state.displays || []).length > 0) {
      const display = state.displays[0];
      preferredDisplayKey = display.key;
      entry = displayEntry(display);
    }

    const wrapper = card("Display override", (node) => node.remove());
    const selectedDisplay = preferredDisplayKey
      ? (state.displays || []).find((display) => display.key === preferredDisplayKey)
      : displayForOverride(entry);

    const hasIdentity = entry.built_in != null || entry.vendor != null || entry.model != null || entry.serial != null;
    const disconnectedEntry = !selectedDisplay && hasIdentity ? entry : null;
    const { target, targetSelect, targetSummary } = buildDisplayTargetBlock(selectedDisplay, disconnectedEntry);
    const { identity, builtIn } = buildIdentityBlock(entry);
    const grid = buildDimensionsGrid(entry);

    function syncDisplayIdentity() {
      const display = selectedDisplayFromSelect(targetSelect);
      if (display) {
        builtIn.value = String(display.built_in);
        wrapper.querySelector('[data-override-field="vendor"]').value = display.vendor ?? "";
        wrapper.querySelector('[data-override-field="model"]').value = display.model ?? "";
        wrapper.querySelector('[data-override-field="serial"]').value = display.serial ?? "";
        targetSummary.textContent = displayIdentitySummary(display);
        identity.hidden = true;
      } else {
        const isDisconnected = targetSelect.value !== "manual";
        identity.hidden = isDisconnected;
        targetSummary.textContent = "";
      }
    }

    wrapper.append(target, identity, grid);
    targetSelect.addEventListener("change", syncDisplayIdentity);
    syncDisplayIdentity();

    displayOverridesEl.append(wrapper);
    if (reveal) {
      wrapper.scrollIntoView({ block: "nearest" });
    }
  }

  function displayKeyFor(entry) {
    return `${entry.built_in}:${entry.vendor ?? ""}:${entry.model ?? ""}:${entry.serial ?? ""}`;
  }

  function displayEntry(display) {
    return {
      built_in: display.built_in,
      vendor: display.vendor,
      model: display.model,
      serial: display.serial,
    };
  }

  function manualDisplayEntry() {
    return {
      built_in: null,
      vendor: null,
      model: null,
      serial: null,
    };
  }

  function firstDisplayWithoutOverride() {
    const used = new Set();
    for (const select of displayOverridesEl.querySelectorAll("[data-display-target]")) {
      if (select.value !== "manual") {
        used.add(select.value);
      }
    }
    return (state.displays || []).find((display) => !used.has(display.key)) || null;
  }

  function handleAddDisplayOverride(event) {
    event?.preventDefault();

    const display = firstDisplayWithoutOverride();
    if (display) {
      addDisplayOverride(displayEntry(display), display.key, true);
      setStatus(`Added display override for ${display.label}.`);
    } else {
      addDisplayOverride(manualDisplayEntry(), "manual", true);
      setStatus("All detected displays already have overrides. Added a manual identity override.");
    }
    updateDirtyState();
  }

  function displayForOverride(entry) {
    const displays = state.displays || [];
    if (entry.built_in != null) {
      const exact = displays.find((display) => display.key === displayKeyFor(entry));
      if (exact) {
        return exact;
      }
    }
    if (entry.serial != null) {
      const serialMatch = displays.find((display) => display.serial === entry.serial);
      if (serialMatch) {
        return serialMatch;
      }
    }
    if (entry.vendor != null && entry.model != null) {
      return displays.find((display) => display.vendor === entry.vendor && display.model === entry.model) || null;
    }
    return null;
  }

  function selectedDisplayFromSelect(select) {
    if (select.value === "manual") {
      return null;
    }
    return (state.displays || []).find((display) => display.key === select.value) || null;
  }

  function displayIdentitySummary(display) {
    const parts = [`${display.built_in ? "Built-in" : "External"}`];
    if (display.vendor != null) {
      parts.push(`vendor ${display.vendor}`);
    }
    if (display.model != null) {
      parts.push(`model ${display.model}`);
    }
    if (display.serial != null) {
      parts.push(`serial ${display.serial}`);
    }
    return parts.join(" · ");
  }

  function collectDisplayOverrides() {
    return Array.from(displayOverridesEl.querySelectorAll(".collection-item")).map((item) => {
      const get = (key) => item.querySelector(`[data-override-field="${key}"]`);
      const selectedDisplay = selectedDisplayFromSelect(item.querySelector("[data-display-target]"));
      const identity = selectedDisplay
        ? {
            built_in: selectedDisplay.built_in,
            vendor: selectedDisplay.vendor,
            model: selectedDisplay.model,
            serial: selectedDisplay.serial,
          }
        : {
            built_in: optionalBool(get("built_in")),
            vendor: optionalNumber(get("vendor"), true),
            model: optionalNumber(get("model"), true),
            serial: optionalNumber(get("serial"), true),
          };
      return {
        ...identity,
        width_fraction: optionalNumber(get("width_fraction")),
        visible_rows: optionalNumber(get("visible_rows"), true),
        min_width: optionalNumber(get("min_width")),
        max_width: optionalNumber(get("max_width")),
        min_height: optionalNumber(get("min_height")),
        max_height: optionalNumber(get("max_height")),
        scale: optionalNumber(get("scale")),
      };
    });
  }

  function renderProviderBoosts(boosts = {}) {
    providerBoostsEl.replaceChildren();
    for (const provider of Object.keys(boosts).sort()) {
      addProviderBoost(provider, boosts[provider]);
    }
  }

  function addProviderBoost(provider = (state.known_providers || [])[0] || "apps", boost = 0) {
    const wrapper = card("Provider boost", (node) => node.remove());
    const grid = document.createElement("div");
    grid.className = "grid two";

    const providerLabel = document.createElement("label");
    const providerText = document.createElement("span");
    const select = document.createElement("select");
    providerText.textContent = "Provider";
    select.dataset.boostProvider = "true";
    for (const known of state.known_providers || []) {
      select.append(option(known));
    }
    select.value = provider;
    providerLabel.append(providerText, select);

    const [boostLabel, boostInput] = labeledInput("Boost", "number");
    boostInput.dataset.boostValue = "true";
    boostInput.step = "1";
    boostInput.value = boost;

    grid.append(providerLabel, boostLabel);
    wrapper.append(grid);
    providerBoostsEl.append(wrapper);
  }

  function collectProviderBoosts() {
    const boosts = {};
    for (const item of providerBoostsEl.querySelectorAll(".collection-item")) {
      const provider = item.querySelector("[data-boost-provider]").value;
      const boost = Math.trunc(Number(item.querySelector("[data-boost-value]").value));
      boosts[provider] = boost;
    }
    return boosts;
  }

  function renderScoreRules(rules = []) {
    scoreRulesEl.replaceChildren();
    for (const rule of rules) {
      addScoreRule(rule);
    }
  }

  function addScoreRule(rule = {}) {
    const wrapper = card("Score rule", (node) => node.remove());
    const grid = document.createElement("div");
    grid.className = "grid five";

    const providersGroup = document.createElement("div");
    providersGroup.className = "checkbox-group";
    providersGroup.dataset.ruleField = "providers";
    const providersGroupLabel = document.createElement("span");
    providersGroupLabel.textContent = "Providers";
    providersGroup.append(providersGroupLabel);
    const selected = new Set(rule.providers || []);
    for (const known of state.known_providers || []) {
      const cb = document.createElement("input");
      cb.type = "checkbox";
      cb.value = known;
      cb.checked = selected.has(known);
      const lbl = document.createElement("label");
      lbl.className = "checkbox-label";
      const txt = document.createElement("span");
      txt.textContent = known;
      lbl.append(cb, txt);
      providersGroup.append(lbl);
    }

    const fieldLabel = document.createElement("label");
    const fieldText = document.createElement("span");
    const fieldSelect = document.createElement("select");
    fieldText.textContent = "Field";
    fieldSelect.dataset.ruleField = "field";
    for (const name of ["title", "subtitle", "badge", "id"]) {
      fieldSelect.append(option(name));
    }
    fieldSelect.value = rule.field || "title";
    fieldLabel.append(fieldText, fieldSelect);

    const matchLabel = document.createElement("label");
    const matchText = document.createElement("span");
    const matchSelect = document.createElement("select");
    matchText.textContent = "Match";
    matchSelect.dataset.ruleField = "match_kind";
    for (const name of ["contains", "prefix", "exact"]) {
      matchSelect.append(option(name));
    }
    matchSelect.value = rule.match_kind || "contains";
    matchLabel.append(matchText, matchSelect);

    const [patternLabel, patternInput] = labeledInput("Pattern", "text");
    patternInput.dataset.ruleField = "pattern";
    patternInput.value = rule.pattern || "";

    const [boostLabel, boostInput] = labeledInput("Boost", "number");
    boostInput.dataset.ruleField = "boost";
    boostInput.step = "1";
    boostInput.value = rule.boost ?? 0;

    grid.append(providersGroup, fieldLabel, matchLabel, patternLabel, boostLabel);
    wrapper.append(grid);
    scoreRulesEl.append(wrapper);
  }

  function collectScoreRules() {
    return Array.from(scoreRulesEl.querySelectorAll(".collection-item")).map((item) => {
      const get = (key) => item.querySelector(`[data-rule-field="${key}"]`);
      return {
        providers: Array.from(get("providers").querySelectorAll("input:checked")).map((cb) => cb.value),
        field: get("field").value,
        match_kind: get("match_kind").value,
        pattern: get("pattern").value.trim(),
        boost: Math.trunc(Number(get("boost").value)),
      };
    });
  }

  function renderCustomColorschemes(schemes = []) {
    colorschemesEl.replaceChildren();
    for (const scheme of schemes) {
      addColorscheme(scheme);
    }
    syncColorschemeOptions();
  }

  function nextColorschemeName() {
    const existing = new Set(collectColorschemes().map((scheme) => scheme.name));
    let index = 1;
    let name = "custom";
    while (existing.has(name)) {
      index += 1;
      name = `custom_${index}`;
    }
    return name;
  }

  function addColorscheme(scheme = {}) {
    const wrapper = card("Custom colorscheme", (node) => {
      node.remove();
      syncColorschemeOptions();
    });
    const top = document.createElement("div");
    top.className = "grid two";

    const [nameLabel, nameInput] = labeledInput("Name", "text");
    nameInput.dataset.colorschemeName = "true";
    nameInput.value = scheme.name || nextColorschemeName();
    nameInput.addEventListener("input", syncColorschemeOptions);

    const baseLabel = document.createElement("label");
    const baseText = document.createElement("span");
    const baseSelect = document.createElement("select");
    baseText.textContent = "Base";
    baseSelect.dataset.colorschemeBase = "true";
    baseSelect.append(option("", "No base"));
    for (const builtin of state.builtin_colorschemes || []) {
      baseSelect.append(option(builtin));
    }
    baseSelect.value = scheme.base || "builtin_dark";
    baseLabel.append(baseText, baseSelect);
    top.append(nameLabel, baseLabel);

    const tokens = document.createElement("div");
    tokens.className = "token-grid";
    for (const token of state.color_tokens || []) {
      const [tokenLabel, tokenInput] = labeledInput(token, "text");
      tokenInput.dataset.colorToken = token;
      tokenInput.value = scheme.tokens?.[token] || "";
      tokens.append(tokenLabel);
    }

    function updateInheritedTokenPlaceholders() {
      const preset = state.color_presets?.[baseSelect.value] || {};
      for (const input of tokens.querySelectorAll("[data-color-token]")) {
        input.placeholder = preset[input.dataset.colorToken] || "";
      }
    }

    baseSelect.addEventListener("change", updateInheritedTokenPlaceholders);
    updateInheritedTokenPlaceholders();

    wrapper.append(top, tokens);
    colorschemesEl.append(wrapper);
    syncColorschemeOptions();
  }

  function collectColorschemes() {
    return Array.from(colorschemesEl.querySelectorAll(".collection-item")).map((item) => {
      const tokens = {};
      for (const input of item.querySelectorAll("[data-color-token]")) {
        const value = input.value.trim();
        if (value) {
          tokens[input.dataset.colorToken] = value;
        }
      }
      return {
        name: item.querySelector("[data-colorscheme-name]").value.trim(),
        base: item.querySelector("[data-colorscheme-base]").value,
        tokens,
      };
    });
  }

  function render(payload) {
    isRendering = true;
    try {
      state = payload;
      configPathEl.textContent = payload.config_path || "";
      rawEl.value = payload.raw || "";
      renderProviders(payload);
      renderColorschemes(payload);

      const hasDraft = !!payload.draft;
      document.body.classList.toggle("config-invalid", !hasDraft);
      if (hasDraft) {
        renderDraft(payload.draft);
        renderDisplayOverrides(payload.draft.display_overrides);
        renderProviderOrder(payload.draft.ranking.provider_order);
        renderProviderBoosts(payload.draft.ranking.provider_score_boosts);
        renderScoreRules(payload.draft.ranking.score_rules);
        renderCustomColorschemes(payload.draft.ui.colorschemes);
        setStatus("");
      } else {
        renderDisplayOverrides([]);
        renderProviderOrder([]);
        renderProviderBoosts({});
        renderScoreRules([]);
        renderCustomColorschemes([]);
        setPane("raw");
        setStatus(payload.error || "Config is invalid", true);
      }
      for (const input of formEl.querySelectorAll("input, select, textarea, button")) {
        input.disabled = !hasDraft;
      }
    } finally {
      captureCleanSnapshots();
      isRendering = false;
      updateSaveButtonState();
    }
  }

  function collectDraft() {
    return {
      debug_log: bool("debug_log"),
      hotkey: {
        shortcut: text("hotkey.shortcut"),
      },
      window: {
        width_fraction: float("window.width_fraction"),
        visible_rows: int("window.visible_rows"),
        min_width: optionalFloat("window.min_width"),
        max_width: optionalFloat("window.max_width"),
        min_height: optionalFloat("window.min_height"),
        max_height: optionalFloat("window.max_height"),
        hide_when_inactive: bool("window.hide_when_inactive"),
        always_on_top: bool("window.always_on_top"),
        show_on: text("window.show_on"),
        scale: float("window.scale"),
      },
      display_overrides: collectDisplayOverrides(),
      providers: {
        disabled: checkedProviders(),
        windows: {
          include_other_desktops: bool("providers.windows.include_other_desktops"),
          show_on_empty_query: bool("providers.windows.show_on_empty_query"),
        },
        apps: {
          exact_name_boost: int("providers.apps.exact_name_boost"),
          prefix_name_boost: int("providers.apps.prefix_name_boost"),
        },
      },
      ranking: {
        tie_threshold: int("ranking.tie_threshold"),
        provider_order: collectProviderOrder(),
        provider_score_boosts: collectProviderBoosts(),
        score_rules: collectScoreRules(),
        result_limit: int("ranking.result_limit"),
      },
      timing: {
        search_debounce_ms: int("timing.search_debounce_ms"),
      },
      plugins: {
        directories: lineList("plugins.directories"),
        search_paths: lineList("plugins.search_paths"),
        plugin_toml: rawText("plugins.plugin_toml"),
      },
      ui: {
        show_header: bool("ui.show_header"),
        cycle_selection: bool("ui.cycle_selection"),
        colorscheme: text("ui.colorscheme"),
        font_family: field("ui.font_family").value.trim(),
        canvas: {
          show: bool("ui.canvas.show"),
          radius: int("ui.canvas.radius"),
          background_opacity: float("ui.canvas.background_opacity"),
          chrome_opacity: float("ui.canvas.chrome_opacity"),
        },
        entries: {
          opacity: float("ui.entries.opacity"),
        },
        shortcuts: {
          focus_window: text("ui.shortcuts.focus_window"),
          activate_all_windows: text("ui.shortcuts.activate_all_windows"),
        },
        colorschemes: collectColorschemes(),
        font_sizes: {
          label: int("ui.font_sizes.label"),
          input: int("ui.font_sizes.input"),
          title: int("ui.font_sizes.title"),
          subtitle: int("ui.font_sizes.subtitle"),
          badge: int("ui.font_sizes.badge"),
          accelerator: int("ui.font_sizes.accelerator"),
          config_error_title: int("ui.font_sizes.config_error_title"),
          config_error_body: int("ui.font_sizes.config_error_body"),
        },
        layout: {
          section_gap: int("ui.layout.section_gap"),
          input_padding_y: int("ui.layout.input_padding_y"),
          input_padding_x: int("ui.layout.input_padding_x"),
          input_radius: int("ui.layout.input_radius"),
          list_gap: int("ui.layout.list_gap"),
          entry_padding_y: int("ui.layout.entry_padding_y"),
          entry_padding_x: int("ui.layout.entry_padding_x"),
          entry_gap: int("ui.layout.entry_gap"),
          row_radius: int("ui.layout.row_radius"),
          badge_size: int("ui.layout.badge_size"),
          badge_radius: int("ui.layout.badge_radius"),
          icon_size: int("ui.layout.icon_size"),
        },
      },
    };
  }

  for (const item of navItems) {
    item.addEventListener("click", () => setPane(item.dataset.pane));
  }

  formEl.addEventListener("input", updateDirtyState);
  formEl.addEventListener("change", updateDirtyState);
  rawEl.addEventListener("input", updateDirtyState);

  for (const input of document.querySelectorAll('input[type="number"]')) {
    normalizeNumberInput(input);
  }

  for (const button of formEl.querySelectorAll("[data-shortcut-recorder]")) {
    button.addEventListener("click", () => {
      if (button.disabled) {
        return;
      }
      recordingShortcut = button;
      button.focus();
      syncShortcutRecorders();
    });
    button.addEventListener("keydown", (event) => {
      if (recordingShortcut !== button) {
        return;
      }
      event.preventDefault();
      event.stopPropagation();
      recordShortcut(button, event);
    });
    button.addEventListener("blur", () => {
      if (recordingShortcut === button) {
        finishShortcutRecording();
      }
    });
  }

  for (const button of formEl.querySelectorAll("[data-shortcut-clear]")) {
    button.addEventListener("click", () => {
      if (button.disabled) {
        return;
      }
      field(button.dataset.shortcutClear).value = "none";
      syncShortcutRecorders();
      updateDirtyState();
    });
  }

  document.getElementById("add-display-override").addEventListener("click", handleAddDisplayOverride);

  document.getElementById("add-provider-boost").addEventListener("click", () => {
    addProviderBoost();
    updateDirtyState();
  });

  document.getElementById("add-score-rule").addEventListener("click", () => {
    addScoreRule();
    updateDirtyState();
  });

  document.getElementById("add-colorscheme").addEventListener("click", () => {
    addColorscheme({ name: nextColorschemeName(), base: "builtin_dark", tokens: {} });
    updateDirtyState();
  });

  document.getElementById("update-plugins-btn").addEventListener("click", () => {
    send({ type: "update_plugins" });
  });

  function reloadSettings() {
    setStatus("Reloading...");
    send({ type: "reload" });
  }

  function closeSettings() {
    send({ type: "close" });
  }

  function isPlainCommandShortcut(event, code) {
    return (
      event.code === code &&
      event.metaKey &&
      !event.ctrlKey &&
      !event.altKey &&
      !event.shiftKey &&
      !recordingShortcut
    );
  }

  document.getElementById("reload").addEventListener("click", reloadSettings);

  function saveActiveEditor() {
    if (saveButton.disabled) {
      return;
    }
    isSaving = true;
    updateSaveButtonState();
    setStatus("Saving...");
    if (activePane === "raw") {
      send({ type: "save_raw", raw: rawEl.value });
    } else {
      const validationError = validateStructuredForm();
      if (validationError) {
        isSaving = false;
        setStatus(validationError, true);
        return;
      }
      send({ type: "save", draft: collectDraft() });
    }
  }

  document.getElementById("save").addEventListener("click", saveActiveEditor);

  document.addEventListener("keydown", (event) => {
    if (isPlainCommandShortcut(event, "KeyS")) {
      event.preventDefault();
      saveActiveEditor();
    } else if (isPlainCommandShortcut(event, "KeyR")) {
      event.preventDefault();
      reloadSettings();
    }
  });

  document.getElementById("close").addEventListener("click", closeSettings);

  root.__RUNX_SETTINGS_STATE__ = render;
  root.__RUNX_SETTINGS_STATUS__ = setStatus;
  render(state);
  send({ type: "ready" });
})(window);
