import type {
  ColorschemeEntry,
  DisplayOptionPayload,
  DisplayOverride,
  PluginInstallEntry,
  ScoreRule,
  SettingsDraft,
  SettingsMessage,
  SettingsPayload,
} from "../types";

function send(payload: SettingsMessage): void {
  window.ipc.postMessage(JSON.stringify(payload));
}

document.addEventListener("contextmenu", (event) => event.preventDefault());
document.addEventListener("click", (e) => {
  const link = (e.target as HTMLElement).closest(
    "[data-url]",
  ) as HTMLElement | null;
  if (link) {
    e.preventDefault();
    send({ type: "open_url", url: link.dataset.url! });
  }
});

const panes = Array.from(
  document.querySelectorAll("[data-pane-panel]"),
) as HTMLElement[];
const navItems = Array.from(
  document.querySelectorAll("[data-pane]"),
) as HTMLElement[];
const statusEl = document.getElementById("status")!;
const saveButton = document.getElementById("save") as HTMLButtonElement;
const formEl = document.getElementById("settings-form") as HTMLFormElement;
const rawEl = document.getElementById("raw-toml") as HTMLTextAreaElement;
const providerListEl = document.getElementById("provider-list")!;
const providerOrderEl = document.getElementById("provider-order-list")!;
const displayOverridesEl = document.getElementById("display-overrides")!;
const providerBoostsEl = document.getElementById("provider-score-boosts")!;
const scoreRulesEl = document.getElementById("score-rules")!;
const pluginInstallEl = document.getElementById("plugin-install-entries")!;
const colorschemesEl = document.getElementById("custom-colorschemes")!;
const configPathEl = document.getElementById("config-path")!;

let state = window.__RUNX_INITIAL_SETTINGS__!;
let activePane = "general";
let cleanSnapshots: { raw: string | null; structured: string | null } = {
  raw: null,
  structured: null,
};
let isRendering = false;
let isSaving = false;

function field(
  path: string,
): HTMLInputElement | HTMLSelectElement | HTMLTextAreaElement {
  return document.querySelector(`[data-field="${path}"]`) as HTMLInputElement;
}

function setStatus(message: string, isError = false): void {
  statusEl.textContent = message || "";
  statusEl.classList.toggle("visible", !!message);
  statusEl.classList.toggle("error", !!isError);
  if (!isRendering && message !== "Saving...") {
    isSaving = false;
    updateSaveButtonState();
  }
}

function setPane(name: string): void {
  activePane = name;
  for (const item of navItems) {
    item.classList.toggle(
      "active",
      (item as HTMLElement).dataset.pane === name,
    );
  }
  formEl.style.display = name === "raw" ? "none" : "";
  for (const pane of panes) {
    pane.classList.toggle("active", pane.dataset.panePanel === name);
  }
  formEl.scrollTop = 0;
  updateDirtyState();
}

function activeEditorKey(): "raw" | "structured" {
  return activePane === "raw" ? "raw" : "structured";
}

function editorSnapshot(
  key: "raw" | "structured" = activeEditorKey(),
): string | null {
  if (key === "raw") {
    return JSON.stringify(rawEl.value);
  }
  if (!state?.draft) {
    return null;
  }
  return JSON.stringify(collectDraft());
}

function captureCleanSnapshots(): void {
  cleanSnapshots = {
    raw: editorSnapshot("raw"),
    structured: state?.draft ? editorSnapshot("structured") : null,
  };
}

function activeEditorCanSave(): boolean {
  return activeEditorKey() === "raw" || !!state?.draft;
}

function activeEditorIsDirty(): boolean {
  const key = activeEditorKey();
  return (
    cleanSnapshots[key] != null && editorSnapshot(key) !== cleanSnapshots[key]
  );
}

function updateSaveButtonState(): void {
  saveButton.disabled =
    isSaving || !activeEditorCanSave() || !activeEditorIsDirty();
}

function updateDirtyState(): void {
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
  } else if (
    statusEl.textContent === "Unsaved changes" ||
    statusEl.textContent === "Saving..."
  ) {
    setStatus("");
  }
}

function pathValue(object: unknown, path: string): unknown {
  return path.split(".").reduce((value: unknown, part: string) => {
    if (value == null || typeof value !== "object") return undefined;
    return (value as Record<string, unknown>)[part];
  }, object);
}

function setField(path: string, value: unknown): void {
  const input = field(path);
  if (!input) {
    return;
  }
  if ((input as HTMLInputElement).type === "checkbox") {
    (input as HTMLInputElement).checked = !!value;
  } else if (Array.isArray(value)) {
    input.value =
      input.tagName === "TEXTAREA" ? value.join("\n") : value.join(", ");
  } else {
    input.value = (value ?? "") as string;
  }
}

function bool(path: string): boolean {
  return !!(field(path) as HTMLInputElement).checked;
}

function text(path: string): string {
  return field(path).value.trim();
}

function rawText(path: string): string {
  return field(path).value;
}

function lineList(path: string): string[] {
  return rawText(path)
    .split(/\r?\n/)
    .map((item) => item.trim())
    .filter(Boolean);
}

function float(path: string): number {
  return Number(field(path).value);
}

function int(path: string): number {
  return Math.trunc(Number(field(path).value));
}

function optionalNumber(
  input: HTMLInputElement,
  integer = false,
): number | null {
  const value = input.value.trim();
  if (!value) {
    return null;
  }
  const parsed = Number(value);
  return integer ? Math.trunc(parsed) : parsed;
}

function optionalFloat(path: string): number | null {
  return optionalNumber(field(path) as HTMLInputElement);
}

function optionalBool(select: HTMLSelectElement): boolean | null {
  if (select.value === "") {
    return null;
  }
  return select.value === "true";
}

function normalizeNumberInput(input: HTMLInputElement): void {
  if (input.type !== "number") {
    return;
  }
  input.type = "text";
  input.inputMode = "decimal";
}

let recordingShortcut: HTMLElement | null = null;

function modifierParts(event: KeyboardEvent): string[] {
  const parts: string[] = [];
  if (event.ctrlKey) parts.push("Ctrl");
  if (event.altKey) parts.push("Option");
  if (event.shiftKey) parts.push("Shift");
  if (event.metaKey) parts.push("Cmd");
  return parts;
}

function modifierDisplay(value: string): string {
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

function displayKeyToken(token: string): string {
  if (!token) return "";
  if (/^Key[A-Z]$/.test(token)) return token.slice(3);
  if (/^Digit[0-9]$/.test(token)) return token.slice(5);
  if (token === "ArrowUp") return "Up";
  if (token === "ArrowDown") return "Down";
  if (token === "ArrowLeft") return "Left";
  if (token === "ArrowRight") return "Right";
  return token;
}

function displayShortcutText(value: string, emptyLabel = "Disabled"): string {
  const textValue = (value || "").trim();
  if (!textValue) return emptyLabel;
  if (/^(none|disabled|off)$/i.test(textValue)) return "Disabled";
  return textValue
    .split("+")
    .map((part) => part.trim())
    .filter(Boolean)
    .map((part) => modifierDisplay(displayKeyToken(part)))
    .join("+");
}

function isModifierOnlyKey(event: KeyboardEvent): boolean {
  return ["Alt", "Control", "Meta", "Shift"].includes(event.key);
}

function shortcutKeyFromEvent(event: KeyboardEvent): string | null {
  const code = event.code || "";
  if (
    /^Key[A-Z]$/.test(code) ||
    /^Digit[0-9]$/.test(code) ||
    /^Numpad[0-9]$/.test(code)
  ) {
    return code;
  }
  if (
    [
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
    ].includes(code)
  ) {
    return code;
  }
  return null;
}

function syncShortcutRecorders(): void {
  for (const button of formEl.querySelectorAll(
    "[data-shortcut-recorder]",
  ) as NodeListOf<HTMLElement>) {
    if (button === recordingShortcut) {
      button.textContent = "Press shortcut...";
      button.classList.add("recording");
      button.classList.remove("empty");
      continue;
    }
    const target = button.dataset.shortcutRecorder!;
    const textValue = displayShortcutText(
      field(target).value,
      "Record shortcut",
    );
    button.textContent = textValue;
    button.classList.toggle(
      "empty",
      textValue === "Disabled" || textValue === "Record shortcut",
    );
    button.classList.remove("recording");
  }
}

function finishShortcutRecording(): void {
  recordingShortcut = null;
  syncShortcutRecorders();
}

function recordShortcut(button: HTMLElement, event: KeyboardEvent): void {
  if (event.key === "Escape" && modifierParts(event).length === 0) {
    finishShortcutRecording();
    return;
  }
  if (isModifierOnlyKey(event)) return;

  const target = button.dataset.shortcutRecorder!;
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

function validateStructuredForm(): string | null {
  const resultLimit = field("ranking.result_limit") as HTMLInputElement;
  if (resultLimit && !resultLimit.disabled && Number(resultLimit.value) < 1) {
    return "Result limit must be at least 1.";
  }

  const colorschemeNames = new Set<string>();
  for (const input of colorschemesEl.querySelectorAll(
    "[data-colorscheme-name]",
  ) as NodeListOf<HTMLInputElement>) {
    const name = input.value.trim();
    if (colorschemeNames.has(name)) {
      return `Custom colorscheme ${name} is duplicated.`;
    }
    colorschemeNames.add(name);
  }

  const boostProviders = new Set<string>();
  for (const select of providerBoostsEl.querySelectorAll(
    "[data-boost-provider]",
  ) as NodeListOf<HTMLSelectElement>) {
    if (boostProviders.has(select.value)) {
      return `Provider score boost for ${select.value} is duplicated.`;
    }
    boostProviders.add(select.value);
  }

  const displayOverrideTargets = new Set<string>();
  for (const select of displayOverridesEl.querySelectorAll(
    "[data-display-target]",
  ) as NodeListOf<HTMLSelectElement>) {
    if (select.value === "manual") continue;
    if (displayOverrideTargets.has(select.value)) {
      return "Display override targets must not be duplicated.";
    }
    displayOverrideTargets.add(select.value);
  }

  return null;
}

function option(value: string, label = value): HTMLOptionElement {
  const node = document.createElement("option");
  node.value = value;
  node.textContent = label;
  return node;
}

function actionButton(label: string, action: () => void): HTMLButtonElement {
  const button = document.createElement("button");
  button.type = "button";
  button.className = "button compact";
  button.textContent = label;
  button.addEventListener("click", action);
  return button;
}

function card(
  title: string,
  onRemove: (node: HTMLElement) => void,
): HTMLElement {
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

function labeledInput(
  label: string,
  type: string,
  className?: string,
): [HTMLLabelElement, HTMLInputElement] {
  const wrapper = document.createElement("label");
  const span = document.createElement("span");
  const input = document.createElement("input");
  span.textContent = label;
  input.type = type;
  normalizeNumberInput(input);
  if (className) input.className = className;
  wrapper.append(span, input);
  return [wrapper, input];
}

function checkedProviders(): string[] {
  return Array.from(
    providerListEl.querySelectorAll("input") as NodeListOf<HTMLInputElement>,
  )
    .filter((input) => !input.checked)
    .map((input) => input.value);
}

function renderProviders(payload: SettingsPayload): void {
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

function renderProviderOrder(order: string[]): void {
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

function collectProviderOrder(): string[] {
  return Array.from(
    providerOrderEl.querySelectorAll(".order-item") as NodeListOf<HTMLElement>,
  ).map((el) => el.dataset.provider!);
}

function syncColorschemeOptions(): void {
  const select = field("ui.colorscheme") as HTMLSelectElement;
  const previous = select.value || state.draft?.ui?.colorscheme || "system";
  const known = new Set(state.colorschemes || []);
  const editedNames = collectColorschemes()
    .map((scheme) => scheme.name)
    .filter(Boolean);
  const names = [
    ...(state.colorschemes || []),
    ...editedNames.filter((name) => !known.has(name)),
  ];
  select.replaceChildren();
  for (const name of names) {
    select.append(option(name));
  }
  select.value = names.includes(previous) ? previous : "system";
}

function renderColorschemes(payload: SettingsPayload): void {
  const select = field("ui.colorscheme") as HTMLSelectElement;
  select.replaceChildren();
  for (const name of payload.colorschemes || []) {
    select.append(option(name));
  }
}

function renderDraft(draft: SettingsDraft): void {
  for (const input of formEl.querySelectorAll(
    "[data-field]",
  ) as NodeListOf<HTMLElement>) {
    setField(
      (input as HTMLElement).dataset.field!,
      pathValue(draft, (input as HTMLElement).dataset.field!),
    );
  }
  syncShortcutRecorders();
}

function renderDisplayOverrides(overrides: DisplayOverride[] = []): void {
  displayOverridesEl.replaceChildren();
  for (const entry of overrides) {
    addDisplayOverride(entry);
  }
}

function displayKeyFor(entry: DisplayOverride): string {
  return `${entry.built_in}:${entry.vendor ?? ""}:${entry.model ?? ""}:${entry.serial ?? ""}`;
}

function displayEntry(display: DisplayOptionPayload): DisplayOverride {
  return {
    built_in: display.built_in,
    vendor: display.vendor,
    model: display.model,
    serial: display.serial,
  };
}

function manualDisplayEntry(): DisplayOverride {
  return { built_in: null, vendor: null, model: null, serial: null };
}

function displayForOverride(
  entry: DisplayOverride,
): DisplayOptionPayload | null {
  const displays = state.displays || [];
  if (entry.built_in != null) {
    const exact = displays.find(
      (display) => display.key === displayKeyFor(entry),
    );
    if (exact) return exact;
  }
  if (entry.serial != null) {
    const serialMatch = displays.find(
      (display) => display.serial === entry.serial,
    );
    if (serialMatch) return serialMatch;
  }
  if (entry.vendor != null && entry.model != null) {
    return (
      displays.find(
        (display) =>
          display.vendor === entry.vendor && display.model === entry.model,
      ) || null
    );
  }
  return null;
}

function selectedDisplayFromSelect(
  select: HTMLSelectElement,
): DisplayOptionPayload | null {
  if (select.value === "manual") return null;
  return (
    (state.displays || []).find((display) => display.key === select.value) ||
    null
  );
}

function displayIdentitySummary(display: DisplayOptionPayload): string {
  const parts = [`${display.built_in ? "Built-in" : "External"}`];
  if (display.vendor != null) parts.push(`vendor ${display.vendor}`);
  if (display.model != null) parts.push(`model ${display.model}`);
  if (display.serial != null) parts.push(`serial ${display.serial}`);
  return parts.join(" · ");
}

function disconnectedDisplayLabel(entry: DisplayOverride): string {
  const parts: string[] = [];
  if (entry.built_in != null)
    parts.push(entry.built_in ? "Built-in" : "External");
  if (entry.vendor != null) parts.push(`vendor ${entry.vendor}`);
  if (entry.model != null) parts.push(`model ${entry.model}`);
  if (entry.serial != null) parts.push(`serial ${entry.serial}`);
  const label = parts.length > 0 ? parts.join(" · ") : "Unknown display";
  return `${label} (disconnected)`;
}

function buildDisplayTargetBlock(
  selectedDisplay: DisplayOptionPayload | null,
  disconnectedEntry: DisplayOverride | null,
) {
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

function buildIdentityBlock(entry: DisplayOverride) {
  const identity = document.createElement("div");
  identity.className = "manual-identity grid four";

  const builtInLabel = document.createElement("label");
  const builtInText = document.createElement("span");
  const builtIn = document.createElement("select");
  builtIn.dataset.overrideField = "built_in";
  builtInText.textContent = "Built-in";
  builtIn.append(
    option("", "Any"),
    option("true", "Built-in"),
    option("false", "External"),
  );
  builtIn.value = entry.built_in == null ? "" : String(entry.built_in);
  builtInLabel.append(builtInText, builtIn);
  identity.append(builtInLabel);

  for (const [key, label] of [
    ["vendor", "Vendor"],
    ["model", "Model"],
    ["serial", "Serial"],
  ] as const) {
    const [labelNode, input] = labeledInput(label, "text");
    input.dataset.overrideField = key;
    input.dataset.optional = "true";
    input.inputMode = "numeric";
    input.value = entry[key] != null ? String(entry[key]) : "";
    identity.append(labelNode);
  }
  return { identity, builtIn };
}

function buildDimensionsGrid(entry: DisplayOverride) {
  const grid = document.createElement("div");
  grid.className = "grid four";

  const fields: [keyof DisplayOverride, string, boolean][] = [
    ["width_fraction", "Width fraction", false],
    ["visible_rows", "Visible rows", true],
    ["min_width", "Min width", false],
    ["max_width", "Max width", false],
    ["min_height", "Min height", false],
    ["max_height", "Max height", false],
    ["scale", "Scale", false],
  ];

  for (const [key, label, integer] of fields) {
    const [labelNode, input] = labeledInput(label, "number");
    input.dataset.overrideField = key;
    input.dataset.optional = "true";
    input.step = integer ? "1" : "0.01";
    input.value = entry[key] != null ? String(entry[key]) : "";
    grid.append(labelNode);
  }
  return grid;
}

function addDisplayOverride(
  entry: DisplayOverride = manualDisplayEntry(),
  preferredDisplayKey: string | null = null,
  reveal = false,
): void {
  if (
    Object.keys(entry).every(
      (k) => entry[k as keyof DisplayOverride] == null,
    ) &&
    (state.displays || []).length > 0
  ) {
    const display = state.displays[0];
    preferredDisplayKey = display.key;
    entry = displayEntry(display);
  }

  const wrapper = card("Display override", (node) => node.remove());
  const selectedDisplay = preferredDisplayKey
    ? (state.displays || []).find(
        (display) => display.key === preferredDisplayKey,
      ) || null
    : displayForOverride(entry);

  const hasIdentity =
    entry.built_in != null ||
    entry.vendor != null ||
    entry.model != null ||
    entry.serial != null;
  const disconnectedEntry = !selectedDisplay && hasIdentity ? entry : null;
  const { target, targetSelect, targetSummary } = buildDisplayTargetBlock(
    selectedDisplay,
    disconnectedEntry,
  );
  const { identity, builtIn } = buildIdentityBlock(entry);
  const grid = buildDimensionsGrid(entry);

  function syncDisplayIdentity(): void {
    const display = selectedDisplayFromSelect(targetSelect);
    if (display) {
      builtIn.value = String(display.built_in);
      (
        wrapper.querySelector(
          '[data-override-field="vendor"]',
        ) as HTMLInputElement
      ).value = display.vendor != null ? String(display.vendor) : "";
      (
        wrapper.querySelector(
          '[data-override-field="model"]',
        ) as HTMLInputElement
      ).value = display.model != null ? String(display.model) : "";
      (
        wrapper.querySelector(
          '[data-override-field="serial"]',
        ) as HTMLInputElement
      ).value = display.serial != null ? String(display.serial) : "";
      targetSummary.textContent = displayIdentitySummary(display);
      (identity as HTMLElement).hidden = true;
    } else {
      const isDisconnected = targetSelect.value !== "manual";
      (identity as HTMLElement).hidden = isDisconnected;
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

function firstDisplayWithoutOverride(): DisplayOptionPayload | null {
  const used = new Set<string>();
  for (const select of displayOverridesEl.querySelectorAll(
    "[data-display-target]",
  ) as NodeListOf<HTMLSelectElement>) {
    if (select.value !== "manual") used.add(select.value);
  }
  return (
    (state.displays || []).find((display) => !used.has(display.key)) || null
  );
}

function handleAddDisplayOverride(event?: Event): void {
  event?.preventDefault();
  const display = firstDisplayWithoutOverride();
  if (display) {
    addDisplayOverride(displayEntry(display), display.key, true);
    setStatus(`Added display override for ${display.label}.`);
  } else {
    addDisplayOverride(manualDisplayEntry(), "manual", true);
    setStatus(
      "All detected displays already have overrides. Added a manual identity override.",
    );
  }
  updateDirtyState();
}

function collectDisplayOverrides(): DisplayOverride[] {
  return Array.from(
    displayOverridesEl.querySelectorAll(
      ".collection-item",
    ) as NodeListOf<HTMLElement>,
  ).map((item) => {
    const get = (key: string) =>
      item.querySelector(`[data-override-field="${key}"]`) as
        | HTMLInputElement
        | HTMLSelectElement;
    const selectedDisplay = selectedDisplayFromSelect(
      item.querySelector("[data-display-target]") as HTMLSelectElement,
    );
    const identity: Pick<
      DisplayOverride,
      "built_in" | "vendor" | "model" | "serial"
    > = selectedDisplay
      ? {
          built_in: selectedDisplay.built_in,
          vendor: selectedDisplay.vendor,
          model: selectedDisplay.model,
          serial: selectedDisplay.serial,
        }
      : {
          built_in: optionalBool(get("built_in") as HTMLSelectElement),
          vendor: optionalNumber(get("vendor") as HTMLInputElement, true),
          model: optionalNumber(get("model") as HTMLInputElement, true),
          serial: optionalNumber(get("serial") as HTMLInputElement, true),
        };
    return {
      ...identity,
      width_fraction: optionalNumber(get("width_fraction") as HTMLInputElement),
      visible_rows: optionalNumber(
        get("visible_rows") as HTMLInputElement,
        true,
      ),
      min_width: optionalNumber(get("min_width") as HTMLInputElement),
      max_width: optionalNumber(get("max_width") as HTMLInputElement),
      min_height: optionalNumber(get("min_height") as HTMLInputElement),
      max_height: optionalNumber(get("max_height") as HTMLInputElement),
      scale: optionalNumber(get("scale") as HTMLInputElement),
    };
  });
}

function renderProviderBoosts(boosts: Record<string, number> = {}): void {
  providerBoostsEl.replaceChildren();
  for (const provider of Object.keys(boosts).sort()) {
    addProviderBoost(provider, boosts[provider]);
  }
}

function addProviderBoost(
  provider = (state.known_providers || [])[0] || "apps",
  boost = 0,
): void {
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
  boostInput.value = String(boost);

  grid.append(providerLabel, boostLabel);
  wrapper.append(grid);
  providerBoostsEl.append(wrapper);
}

function collectProviderBoosts(): Record<string, number> {
  const boosts: Record<string, number> = {};
  for (const item of providerBoostsEl.querySelectorAll(
    ".collection-item",
  ) as NodeListOf<HTMLElement>) {
    const provider = (
      item.querySelector("[data-boost-provider]") as HTMLSelectElement
    ).value;
    const boost = Math.trunc(
      Number(
        (item.querySelector("[data-boost-value]") as HTMLInputElement).value,
      ),
    );
    boosts[provider] = boost;
  }
  return boosts;
}

function renderScoreRules(rules: ScoreRule[] = []): void {
  scoreRulesEl.replaceChildren();
  for (const rule of rules) {
    addScoreRule(rule);
  }
}

function addScoreRule(rule: Partial<ScoreRule> = {}): void {
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
  boostInput.value = String(rule.boost ?? 0);

  grid.append(providersGroup, fieldLabel, matchLabel, patternLabel, boostLabel);
  wrapper.append(grid);
  scoreRulesEl.append(wrapper);
}

function collectScoreRules(): ScoreRule[] {
  return Array.from(
    scoreRulesEl.querySelectorAll(
      ".collection-item",
    ) as NodeListOf<HTMLElement>,
  ).map((item) => {
    const get = (key: string) =>
      item.querySelector(`[data-rule-field="${key}"]`) as HTMLElement;
    return {
      providers: Array.from(
        (get("providers") as HTMLElement).querySelectorAll(
          "input:checked",
        ) as NodeListOf<HTMLInputElement>,
      ).map((cb) => cb.value),
      field: (get("field") as HTMLSelectElement).value,
      match_kind: (get("match_kind") as HTMLSelectElement).value,
      pattern: (get("pattern") as HTMLInputElement).value.trim(),
      boost: Math.trunc(Number((get("boost") as HTMLInputElement).value)),
    };
  });
}

function renderPluginInstall(entries: PluginInstallEntry[] = []): void {
  pluginInstallEl.replaceChildren();
  for (const entry of entries) {
    addPluginInstall(entry);
  }
}

function addPluginInstall(entry: Partial<PluginInstallEntry> = {}): void {
  const wrapper = document.createElement("div");
  wrapper.className = "collection-item plugin-install-row";

  const [sourceLabel, sourceInput] = labeledInput("Source URL", "text");
  sourceInput.dataset.installField = "source";
  sourceInput.placeholder = "https://github.com/user/plugin.git";
  sourceInput.value = entry.source || "";

  const [nameLabel, nameInput] = labeledInput("Install as", "text");
  nameInput.dataset.installField = "name";
  nameInput.placeholder = "optional directory name override";
  nameInput.value = entry.name || "";

  const [pinLabel, pinInput] = labeledInput(
    "Pin (tag, branch, or commit)",
    "text",
  );
  pinInput.dataset.installField = "pin";
  pinInput.placeholder = "e.g. v1.0.0, main, a1b2c3d";
  pinInput.value = entry.ref || entry.branch || "";

  const deleteBtn = actionButton("Delete", () => {
    wrapper.remove();
    updateDirtyState();
  });

  wrapper.append(sourceLabel, nameLabel, pinLabel, deleteBtn);
  pluginInstallEl.append(wrapper);
}

function collectPluginInstall(): PluginInstallEntry[] {
  return Array.from(
    pluginInstallEl.querySelectorAll(
      ".collection-item",
    ) as NodeListOf<HTMLElement>,
  )
    .map((item) => {
      const get = (key: string) =>
        item.querySelector(
          `[data-install-field="${key}"]`,
        ) as HTMLInputElement | null;
      const entry: PluginInstallEntry = {
        source: (get("source")?.value || "").trim(),
      };
      const name = (get("name")?.value || "").trim();
      if (name) entry.name = name;
      const pin = (get("pin")?.value || "").trim();
      if (pin) entry.ref = pin;
      return entry;
    })
    .filter((entry) => entry.source.length > 0);
}

function renderCustomColorschemes(schemes: ColorschemeEntry[] = []): void {
  colorschemesEl.replaceChildren();
  for (const scheme of schemes) {
    addColorscheme(scheme);
  }
  syncColorschemeOptions();
}

function nextColorschemeName(): string {
  const existing = new Set(collectColorschemes().map((scheme) => scheme.name));
  let index = 1;
  let name = "custom";
  while (existing.has(name)) {
    index += 1;
    name = `custom_${index}`;
  }
  return name;
}

function addColorscheme(scheme: Partial<ColorschemeEntry> = {}): void {
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

  function updateInheritedTokenPlaceholders(): void {
    const preset = state.color_presets?.[baseSelect.value] || {};
    for (const input of tokens.querySelectorAll(
      "[data-color-token]",
    ) as NodeListOf<HTMLInputElement>) {
      input.placeholder = preset[input.dataset.colorToken!] || "";
    }
  }

  baseSelect.addEventListener("change", updateInheritedTokenPlaceholders);
  updateInheritedTokenPlaceholders();

  wrapper.append(top, tokens);
  colorschemesEl.append(wrapper);
  syncColorschemeOptions();
}

function collectColorschemes(): ColorschemeEntry[] {
  return Array.from(
    colorschemesEl.querySelectorAll(
      ".collection-item",
    ) as NodeListOf<HTMLElement>,
  ).map((item) => {
    const tokens: Record<string, string> = {};
    for (const input of item.querySelectorAll(
      "[data-color-token]",
    ) as NodeListOf<HTMLInputElement>) {
      const value = input.value.trim();
      if (value) tokens[input.dataset.colorToken!] = value;
    }
    return {
      name: (
        item.querySelector("[data-colorscheme-name]") as HTMLInputElement
      ).value.trim(),
      base: (item.querySelector("[data-colorscheme-base]") as HTMLSelectElement)
        .value,
      tokens,
    };
  });
}

function render(payload: SettingsPayload): void {
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
      renderDraft(payload.draft!);
      renderDisplayOverrides(payload.draft!.display_overrides);
      renderProviderOrder(payload.draft!.ranking.provider_order);
      renderProviderBoosts(payload.draft!.ranking.provider_score_boosts);
      renderScoreRules(payload.draft!.ranking.score_rules);
      renderPluginInstall(payload.draft!.plugins.install);
      renderCustomColorschemes(payload.draft!.ui.colorschemes);
      setStatus("");
    } else {
      renderDisplayOverrides([]);
      renderProviderOrder([]);
      renderProviderBoosts({});
      renderScoreRules([]);
      renderPluginInstall([]);
      renderCustomColorschemes([]);
      setPane("raw");
      setStatus(payload.error || "Config is invalid", true);
    }
    for (const input of formEl.querySelectorAll(
      "input, select, textarea, button",
    ) as NodeListOf<HTMLInputElement>) {
      input.disabled = !hasDraft;
    }
  } finally {
    captureCleanSnapshots();
    isRendering = false;
    updateSaveButtonState();
  }
}

function collectDraft(): SettingsDraft {
  return {
    debug_log: bool("debug_log"),
    hotkey: {
      shortcut: text("hotkey.shortcut"),
      quick_switch: text("hotkey.quick_switch"),
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
      show_animation: bool("window.show_animation"),
      show_on: text("window.show_on"),
      scale: float("window.scale"),
    },
    display_overrides: collectDisplayOverrides(),
    providers: {
      disabled: checkedProviders(),
      windows: {
        include_other_desktops: bool(
          "providers.windows.include_other_desktops",
        ),
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
      quick_switch_show_delay_ms: int("timing.quick_switch_show_delay_ms"),
    },
    plugins: {
      directories: lineList("plugins.directories"),
      search_paths: lineList("plugins.search_paths"),
      install: collectPluginInstall(),
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
        show_hover: bool("ui.entries.show_hover"),
        transition_ms: int("ui.entries.transition_ms"),
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
  item.addEventListener("click", () =>
    setPane((item as HTMLElement).dataset.pane!),
  );
}

formEl.addEventListener("input", updateDirtyState);
formEl.addEventListener("change", updateDirtyState);
rawEl.addEventListener("input", updateDirtyState);

for (const input of document.querySelectorAll(
  'input[type="number"]',
) as NodeListOf<HTMLInputElement>) {
  normalizeNumberInput(input);
}

for (const button of formEl.querySelectorAll(
  "[data-shortcut-recorder]",
) as NodeListOf<HTMLElement>) {
  button.addEventListener("click", () => {
    if ((button as HTMLButtonElement).disabled) return;
    recordingShortcut = button;
    button.focus();
    syncShortcutRecorders();
  });
  button.addEventListener("keydown", (event: Event) => {
    if (recordingShortcut !== button) return;
    (event as KeyboardEvent).preventDefault();
    (event as KeyboardEvent).stopPropagation();
    recordShortcut(button, event as KeyboardEvent);
  });
  button.addEventListener("blur", () => {
    if (recordingShortcut === button) finishShortcutRecording();
  });
}

for (const button of formEl.querySelectorAll(
  "[data-shortcut-clear]",
) as NodeListOf<HTMLElement>) {
  button.addEventListener("click", () => {
    if ((button as HTMLButtonElement).disabled) return;
    field(button.dataset.shortcutClear!).value = "none";
    syncShortcutRecorders();
    updateDirtyState();
  });
}

document
  .getElementById("add-display-override")!
  .addEventListener("click", handleAddDisplayOverride);

document.getElementById("add-provider-boost")!.addEventListener("click", () => {
  addProviderBoost();
  updateDirtyState();
});

document.getElementById("add-score-rule")!.addEventListener("click", () => {
  addScoreRule();
  updateDirtyState();
});

document.getElementById("add-colorscheme")!.addEventListener("click", () => {
  addColorscheme({
    name: nextColorschemeName(),
    base: "builtin_dark",
    tokens: {},
  });
  updateDirtyState();
});

document.getElementById("add-plugin-install")!.addEventListener("click", () => {
  addPluginInstall();
  updateDirtyState();
});

document.getElementById("update-plugins-btn")!.addEventListener("click", () => {
  send({ type: "update_plugins" });
});

function reloadSettings(): void {
  setStatus("Reloading...");
  send({ type: "reload" });
}

function closeSettings(): void {
  send({ type: "close" });
}

function isPlainCommandShortcut(event: KeyboardEvent, code: string): boolean {
  return (
    event.code === code &&
    event.metaKey &&
    !event.ctrlKey &&
    !event.altKey &&
    !event.shiftKey &&
    !recordingShortcut
  );
}

document.getElementById("reload")!.addEventListener("click", reloadSettings);

function saveActiveEditor(): void {
  if (saveButton.disabled) return;
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

document.getElementById("save")!.addEventListener("click", saveActiveEditor);

document.addEventListener("keydown", (event: KeyboardEvent) => {
  if (isPlainCommandShortcut(event, "KeyS")) {
    event.preventDefault();
    saveActiveEditor();
  } else if (isPlainCommandShortcut(event, "KeyR")) {
    event.preventDefault();
    reloadSettings();
  }
});

document.getElementById("close")!.addEventListener("click", closeSettings);

window.__RUNX_SETTINGS_STATE__ = render;
window.__RUNX_SETTINGS_STATUS__ = setStatus;
render(state);
send({ type: "ready" });
