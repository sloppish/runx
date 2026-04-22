const test = require("node:test");
const assert = require("node:assert/strict");
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

test("typing resets selection to the first item", () => {
  const state = ui.createState();
  state.items = [{}, {}, {}];
  state.selectedIndex = 2;

  ui.inputChanged(state, "photo");

  assert.equal(state.query, "photo");
  assert.equal(state.selectedIndex, 0);
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
