const test = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const ui = require("./app.js");

test("hover does not steal keyboard selection", () => {
  const state = ui.createState();
  state.items = [{}, {}, {}];

  ui.moveSelection(state, 1);
  assert.equal(state.selectedIndex, 1);
});

test("selection can wrap from first to last and last to first", () => {
  const state = ui.createState();
  state.items = [{}, {}, {}];

  ui.moveSelection(state, -1, true);
  assert.equal(state.selectedIndex, 2);

  ui.moveSelection(state, 1, true);
  assert.equal(state.selectedIndex, 0);
});

test("ctrl-n and ctrl-p use physical key codes so they survive non-latin layouts", () => {
  assert.equal(
    ui.isCtrlNextShortcut({ key: "т", code: "KeyN", ctrlKey: true, metaKey: false, altKey: false }),
    true,
  );
  assert.equal(
    ui.isCtrlPreviousShortcut({ key: "з", code: "KeyP", ctrlKey: true, metaKey: false, altKey: false }),
    true,
  );
  assert.equal(
    ui.isCtrlNextShortcut({ key: "n", code: "KeyN", ctrlKey: false, metaKey: false, altKey: false }),
    false,
  );
});

test("copy shortcut uses physical key code in config error mode", () => {
  assert.equal(
    ui.isCopyShortcut({ code: "KeyC", metaKey: true, ctrlKey: false, altKey: false, shiftKey: false }),
    true,
  );
  assert.equal(
    ui.isCopyShortcut({ code: "KeyC", metaKey: false, ctrlKey: true, altKey: false, shiftKey: false }),
    true,
  );
  assert.equal(
    ui.isCopyShortcut({ code: "KeyC", metaKey: true, ctrlKey: false, altKey: true, shiftKey: false }),
    false,
  );
});

test("paste shortcut uses physical key code", () => {
  assert.equal(
    ui.isPasteShortcut({ code: "KeyV", metaKey: true, ctrlKey: false, altKey: false, shiftKey: false }),
    true,
  );
  assert.equal(
    ui.isPasteShortcut({ code: "KeyV", metaKey: false, ctrlKey: true, altKey: false, shiftKey: false }),
    true,
  );
  assert.equal(
    ui.isPasteShortcut({ code: "KeyV", metaKey: true, ctrlKey: false, altKey: false, shiftKey: true }),
    false,
  );
});

test("input copy reads selected text", () => {
  assert.equal(ui.selectedInputText({ value: "abcdef", selectionStart: 1, selectionEnd: 4 }), "bcd");
  assert.equal(ui.selectedInputText({ value: "abcdef", selectionStart: 4, selectionEnd: 1 }), "bcd");
  assert.equal(ui.selectedInputText({ value: "abcdef", selectionStart: 2, selectionEnd: 2 }), "");
});

test("input paste replaces selected text and moves caret", () => {
  const input = {
    value: "abcdef",
    selectionStart: 2,
    selectionEnd: 5,
    setSelectionRange(start, end) {
      this.selectionStart = start;
      this.selectionEnd = end;
    },
  };

  const next = ui.replaceInputSelection(input, "XYZ");

  assert.equal(next, "abXYZf");
  assert.equal(input.value, "abXYZf");
  assert.equal(input.selectionStart, 5);
  assert.equal(input.selectionEnd, 5);
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

  assert.equal(
    ui.matchesShortcut({ key: "Enter", code: "Enter", altKey: true, ctrlKey: false, metaKey: false, shiftKey: false }, shortcut),
    true,
  );
  assert.equal(
    ui.matchesShortcut({ key: "Enter", code: "Enter", altKey: false, ctrlKey: false, metaKey: false, shiftKey: false }, shortcut),
    false,
  );
});

test("typing resets selection to the first item", () => {
  const state = ui.createState();
  state.items = [{}, {}, {}];
  state.selectedIndex = 2;

  ui.inputChanged(state, "photo");

  assert.equal(state.query, "photo");
  assert.equal(state.selectedIndex, 0);
});

test("visible rows normalize to a positive integer", () => {
  assert.equal(ui.normalizeVisibleRows(undefined), 1);
  assert.equal(ui.normalizeVisibleRows(0), 1);
  assert.equal(ui.normalizeVisibleRows("4.6"), 5);
});

test("settings window clamps are optional when left blank", () => {
  const html = fs.readFileSync(path.join(__dirname, "settings.html"), "utf8");

  for (const field of ["min_width", "max_width", "min_height", "max_height"]) {
    const pattern = new RegExp(`<input[^>]*data-optional="true"[^>]*data-field="window\\.${field}"`);
    assert.match(html, pattern);
  }
});

test("backend render payload does not overwrite active typing with stale query text", () => {
  const state = ui.createState();
  state.query = "gmail";
  state.selectedIndex = 1;

  const update = ui.applyRenderPayload(
    state,
    { query: "gma", items: [{ title: "Gmail" }] },
    { inputValue: "gmail", inputFocused: true },
  );

  assert.equal(update.shouldSyncInput, false);
  assert.equal(state.query, "gmail");
  assert.equal(state.items.length, 1);
  assert.equal(state.selectedIndex, 1);
});

test("backend render payload preserves config error separately from items", () => {
  const state = ui.createState();

  ui.applyRenderPayload(
    state,
    { query: "", config_error: "bad syntax", items: [] },
    { inputValue: "", inputFocused: false },
  );

  assert.equal(state.configError, "bad syntax");
  assert.equal(state.items.length, 0);
});
