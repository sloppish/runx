const test = require("node:test");
const assert = require("node:assert/strict");
const ui = require("./app.js");

test("hover does not steal keyboard selection", () => {
  const state = ui.createState();
  state.items = [{}, {}, {}];

  ui.moveSelection(state, 1);
  assert.equal(state.selectedIndex, 1);
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
