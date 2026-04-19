const test = require("node:test");
const assert = require("node:assert/strict");
const ui = require("./app.js");

test("keyboard navigation suspends hover ownership until the mouse moves again", () => {
  const state = ui.createState();
  state.items = [{}, {}, {}];

  assert.equal(ui.hoverRow(state, 1), true);
  assert.equal(state.selectedIndex, 1);

  ui.moveSelection(state, 1);
  assert.equal(state.selectedIndex, 2);
  assert.equal(state.hoverSuspended, true);

  assert.equal(ui.hoverRow(state, 1), false);
  assert.equal(state.selectedIndex, 2);

  ui.resumeHover(state);
  assert.equal(ui.hoverRow(state, 1), true);
  assert.equal(state.selectedIndex, 1);
});

test("typing resets selection to the first item", () => {
  const state = ui.createState();
  state.items = [{}, {}, {}];
  state.selectedIndex = 2;

  ui.inputChanged(state, "photo");

  assert.equal(state.query, "photo");
  assert.equal(state.selectedIndex, 0);
  assert.equal(state.hoverSuspended, true);
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
