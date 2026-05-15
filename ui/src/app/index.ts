import type { RenderPayload, Shortcut } from "../types";
import {
  type DomElements,
  measurePreferredHeight,
  queryElements,
  renderConfigError,
  renderResults,
  syncSelection,
} from "./dom";
import { send } from "./ipc";
import {
  isCopyShortcut,
  isCtrlNextShortcut,
  isCtrlPreviousShortcut,
  isPasteShortcut,
  matchesShortcut,
} from "./shortcuts";
import {
  applyRenderPayload,
  clampSelection,
  createState,
  inputChanged,
  type LauncherState,
  moveSelection,
  normalizeVisibleRows,
} from "./state";
import {
  escapeAttr,
  escapeHtml,
  replaceInputSelection,
  selectedInputText,
} from "./text";

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

if (typeof window !== "undefined" && window.document) {
  const state: LauncherState = createState();
  const dom: DomElements = queryElements();
  let pendingPreferredHeightTimer: ReturnType<typeof setTimeout> | null = null;
  let lastReportedPreferredHeight: number | null = null;
  let lastReportedLayoutVersion: number | null = null;

  function cycleSelectionEnabled(): boolean {
    return !!window.__RUNX_CYCLE_SELECTION__;
  }

  function activateAllWindowsShortcut(): Shortcut | null {
    return window.__RUNX_ACTIVATE_ALL_WINDOWS_SHORTCUT__ || null;
  }

  function focusWindowShortcut(): Shortcut | null {
    return window.__RUNX_FOCUS_WINDOW_SHORTCUT__ || null;
  }

  function inConfigErrorMode(): boolean {
    return !!state.configError && state.query === "";
  }

  function visibleRows(): number {
    return normalizeVisibleRows(window.__RUNX_VISIBLE_ROWS__);
  }

  function layoutVersion(): number {
    const parsed = Number(window.__RUNX_LAYOUT_VERSION__);
    if (!Number.isFinite(parsed) || parsed < 0) {
      return 0;
    }
    return Math.floor(parsed);
  }

  function reportPreferredHeight(): void {
    const height = measurePreferredHeight(dom, visibleRows(), state.mode);
    const version = layoutVersion();
    if (!Number.isFinite(height) || height <= 0) {
      return;
    }
    if (
      lastReportedPreferredHeight === height &&
      lastReportedLayoutVersion === version
    ) {
      return;
    }
    lastReportedPreferredHeight = height;
    lastReportedLayoutVersion = version;
    send({ type: "preferred_height", height, layout_version: version });
  }

  function queuePreferredHeightReport(): void {
    if (pendingPreferredHeightTimer !== null) {
      clearTimeout(pendingPreferredHeightTimer);
    }
    pendingPreferredHeightTimer = setTimeout(() => {
      pendingPreferredHeightTimer = null;
      reportPreferredHeight();
    }, 0);
  }

  function render(): void {
    clampSelection(state);

    if (inConfigErrorMode()) {
      renderConfigError(dom, state.configError!);
      return;
    }

    renderResults(dom, state);
    syncSelection(dom, state.selectedIndex);
  }

  window.__RUNX_RENDER = (payload: RenderPayload) => {
    const update = applyRenderPayload(state, payload, {
      inputValue: dom.input.value,
      inputFocused: document.activeElement === dom.input,
    });

    if (update.shouldSyncInput && dom.input.value !== update.inputValue) {
      dom.input.value = update.inputValue;
    }

    render();
    if (update.modeChanged) {
      queuePreferredHeightReport();
    }
  };

  window.__RUNX_FOCUS = () => {
    if (state.mode === "quick_switch") {
      return;
    }
    if (inConfigErrorMode()) {
      return;
    }
    dom.input.focus();
    dom.input.select();
  };

  window.__RUNX_PASTE_TEXT = (text?: string) => {
    if (inConfigErrorMode()) {
      return;
    }
    dom.input.focus();
    const query = replaceInputSelection(dom.input, text || "");
    inputChanged(state, query);
    syncSelection(dom, state.selectedIndex, false);
    send({ type: "query_changed", query });
  };

  window.__RUNX_REQUEST_PREFERRED_HEIGHT = () => {
    queuePreferredHeightReport();
  };

  dom.input.addEventListener("input", () => {
    inputChanged(state, dom.input.value);
    syncSelection(dom, state.selectedIndex, false);
    send({ type: "query_changed", query: dom.input.value });
  });

  dom.input.addEventListener("keydown", (event: KeyboardEvent) => {
    if (isCopyShortcut(event)) {
      const text = selectedInputText(dom.input);
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

    if (
      event.code === "KeyA" &&
      event.metaKey &&
      !event.altKey &&
      !event.shiftKey &&
      !event.ctrlKey
    ) {
      event.preventDefault();
      dom.input.select();
      return;
    }

    if (isCtrlNextShortcut(event)) {
      event.preventDefault();
      moveSelection(state, 1, cycleSelectionEnabled());
      syncSelection(dom, state.selectedIndex);
      return;
    }

    if (isCtrlPreviousShortcut(event)) {
      event.preventDefault();
      moveSelection(state, -1, cycleSelectionEnabled());
      syncSelection(dom, state.selectedIndex);
      return;
    }

    if (matchesShortcut(event, activateAllWindowsShortcut())) {
      event.preventDefault();
      send({ type: "activate", index: state.selectedIndex, all_windows: true });
      return;
    }

    if (matchesShortcut(event, focusWindowShortcut())) {
      event.preventDefault();
      send({
        type: "activate",
        index: state.selectedIndex,
        all_windows: false,
      });
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

  document.addEventListener("keydown", (event: KeyboardEvent) => {
    if (state.mode === "quick_switch") {
      if (event.altKey && event.code === "Tab") {
        event.preventDefault();
        send({ type: "quick_switch_cycle", delta: event.shiftKey ? -1 : 1 });
        return;
      }
      if (event.key === "Escape") {
        event.preventDefault();
        send({ type: "hide" });
        return;
      }
      if (event.key.length === 1 && !event.metaKey && !event.ctrlKey) {
        event.preventDefault();
      }
      return;
    }

    if (!inConfigErrorMode()) {
      return;
    }

    if (event.key === "Escape") {
      event.preventDefault();
      send({ type: "hide" });
      return;
    }

    if (isCopyShortcut(event)) {
      const selection = window.getSelection ? window.getSelection() : null;
      const text = selection ? selection.toString() : "";
      if (text) {
        event.preventDefault();
        send({ type: "copy_text", text });
      }
      return;
    }

    if (
      event.metaKey ||
      event.ctrlKey ||
      event.altKey ||
      event.key.length !== 1
    ) {
      return;
    }

    event.preventDefault();
    dom.input.value = event.key;
    inputChanged(state, dom.input.value);
    render();
    dom.input.focus();
    send({ type: "query_changed", query: dom.input.value });
  });

  send({ type: "ready" });
  queuePreferredHeightReport();
  window.RunxUi = api;
}

export {
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
