import { test, expect } from "bun:test";
import {
  createState,
  normalizeVisibleRows,
  moveSelection,
  inputChanged,
  applyRenderPayload,
  isCtrlNextShortcut,
  isCtrlPreviousShortcut,
  isCopyShortcut,
  isPasteShortcut,
  matchesShortcut,
  selectedInputText,
  replaceInputSelection,
} from "./src/app/index";

test("hover does not steal keyboard selection", () => {
  const state = createState();
  state.items = [{}, {}, {}] as any;

  moveSelection(state, 1);
  expect(state.selectedIndex).toBe(1);
});

test("selection can wrap from first to last and last to first", () => {
  const state = createState();
  state.items = [{}, {}, {}] as any;

  moveSelection(state, -1, true);
  expect(state.selectedIndex).toBe(2);

  moveSelection(state, 1, true);
  expect(state.selectedIndex).toBe(0);
});

test("ctrl-n and ctrl-p use physical key codes so they survive non-latin layouts", () => {
  expect(
    isCtrlNextShortcut({ key: "т", code: "KeyN", ctrlKey: true, metaKey: false, altKey: false } as any),
  ).toBe(true);
  expect(
    isCtrlPreviousShortcut({ key: "з", code: "KeyP", ctrlKey: true, metaKey: false, altKey: false } as any),
  ).toBe(true);
  expect(
    isCtrlNextShortcut({ key: "n", code: "KeyN", ctrlKey: false, metaKey: false, altKey: false } as any),
  ).toBe(false);
});

test("copy shortcut uses physical key code in config error mode", () => {
  expect(
    isCopyShortcut({ code: "KeyC", metaKey: true, ctrlKey: false, altKey: false, shiftKey: false } as any),
  ).toBe(true);
  expect(
    isCopyShortcut({ code: "KeyC", metaKey: false, ctrlKey: true, altKey: false, shiftKey: false } as any),
  ).toBe(false);
  expect(
    isCopyShortcut({ code: "KeyC", metaKey: true, ctrlKey: false, altKey: true, shiftKey: false } as any),
  ).toBe(false);
});

test("paste shortcut uses physical key code", () => {
  expect(
    isPasteShortcut({ code: "KeyV", metaKey: true, ctrlKey: false, altKey: false, shiftKey: false } as any),
  ).toBe(true);
  expect(
    isPasteShortcut({ code: "KeyV", metaKey: false, ctrlKey: true, altKey: false, shiftKey: false } as any),
  ).toBe(false);
  expect(
    isPasteShortcut({ code: "KeyV", metaKey: true, ctrlKey: false, altKey: false, shiftKey: true } as any),
  ).toBe(false);
});

test("input copy reads selected text", () => {
  expect(selectedInputText({ value: "abcdef", selectionStart: 1, selectionEnd: 4 } as any)).toBe("bcd");
  expect(selectedInputText({ value: "abcdef", selectionStart: 4, selectionEnd: 1 } as any)).toBe("bcd");
  expect(selectedInputText({ value: "abcdef", selectionStart: 2, selectionEnd: 2 } as any)).toBe("");
});

test("input paste replaces selected text and moves caret", () => {
  const input = {
    value: "abcdef",
    selectionStart: 2,
    selectionEnd: 5,
    setSelectionRange(start: number, end: number) {
      this.selectionStart = start;
      this.selectionEnd = end;
    },
  };

  const next = replaceInputSelection(input as any, "XYZ");

  expect(next).toBe("abXYZf");
  expect(input.value).toBe("abXYZf");
  expect(input.selectionStart).toBe(5);
  expect(input.selectionEnd).toBe(5);
});

test("shortcut matcher supports configurable option-enter", () => {
  const shortcut = {
    key: "Enter",
    code: null,
    alt: true,
    ctrl: false,
    meta: false,
    shift: false,
  };

  expect(
    matchesShortcut({ key: "Enter", code: "Enter", altKey: true, ctrlKey: false, metaKey: false, shiftKey: false } as any, shortcut),
  ).toBe(true);
  expect(
    matchesShortcut({ key: "Enter", code: "Enter", altKey: false, ctrlKey: false, metaKey: false, shiftKey: false } as any, shortcut),
  ).toBe(false);
});

test("typing resets selection to the first item", () => {
  const state = createState();
  state.items = [{}, {}, {}] as any;
  state.selectedIndex = 2;

  inputChanged(state, "photo");

  expect(state.query).toBe("photo");
  expect(state.selectedIndex).toBe(0);
});

test("visible rows normalize to a positive integer", () => {
  expect(normalizeVisibleRows(undefined)).toBe(1);
  expect(normalizeVisibleRows(0)).toBe(1);
  expect(normalizeVisibleRows("4.6" as any)).toBe(5);
});


test("backend render payload does not overwrite active typing with stale query text", () => {
  const state = createState();
  state.query = "gmail";
  state.selectedIndex = 1;

  const update = applyRenderPayload(
    state,
    { query: "gma", items: [{ title: "Gmail" }] } as any,
    { inputValue: "gmail", inputFocused: true },
  );

  expect(update.shouldSyncInput).toBe(false);
  expect(state.query).toBe("gmail");
  expect(state.items.length).toBe(1);
  expect(state.selectedIndex).toBe(1);
});

test("backend render payload preserves config error separately from items", () => {
  const state = createState();

  applyRenderPayload(
    state,
    { query: "", config_error: "bad syntax", items: [] },
    { inputValue: "", inputFocused: false },
  );

  expect(state.configError).toBe("bad syntax");
  expect(state.items.length).toBe(0);
});
