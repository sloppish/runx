import type { ActionFeedback, RenderItem, RenderPayload } from "../types";

export interface LauncherState {
  mode: "regular" | "quick_switch";
  query: string;
  configError: string | null;
  actionFeedback: ActionFeedback | null;
  items: RenderItem[];
  selectedIndex: number;
}

export interface Environment {
  inputValue: string;
  inputFocused: boolean;
}

export interface RenderUpdate {
  shouldSyncInput: boolean;
  inputValue: string;
  modeChanged: boolean;
}

export function createState(): LauncherState {
  return {
    mode: "regular",
    query: "",
    configError: null,
    actionFeedback: null,
    items: [],
    selectedIndex: 0,
  };
}

export function normalizeVisibleRows(value: unknown): number {
  const parsed = Number(value);
  if (!Number.isFinite(parsed)) {
    return 1;
  }
  return Math.max(1, Math.round(parsed));
}

export function clampSelection(state: LauncherState): void {
  if (state.items.length === 0) {
    state.selectedIndex = 0;
    return;
  }
  state.selectedIndex = Math.max(
    0,
    Math.min(state.selectedIndex, state.items.length - 1),
  );
}

export function moveSelection(
  state: LauncherState,
  delta: number,
  cycle = false,
): void {
  if (state.items.length === 0) {
    return;
  }

  if (!cycle) {
    state.selectedIndex = Math.max(
      0,
      Math.min(state.selectedIndex + delta, state.items.length - 1),
    );
    return;
  }

  const count = state.items.length;
  state.selectedIndex =
    (((state.selectedIndex + delta) % count) + count) % count;
}

export function inputChanged(state: LauncherState, query: string): void {
  state.query = query;
  state.selectedIndex = 0;
}

export function applyRenderPayload(
  state: LauncherState,
  payload: RenderPayload,
  environment: Environment,
): RenderUpdate {
  const payloadQuery = typeof payload.query === "string" ? payload.query : "";
  const nextMode = payload.mode === "quick_switch" ? "quick_switch" : "regular";
  const modeChanged = state.mode !== nextMode;
  state.mode = nextMode;
  state.configError =
    typeof payload.config_error === "string" ? payload.config_error : null;
  state.actionFeedback = payload.action_feedback ?? null;
  const shouldSyncInput =
    payloadQuery === "" ||
    environment.inputValue === payloadQuery ||
    !environment.inputFocused;
  const queryChanged = state.query !== payloadQuery;

  if (shouldSyncInput) {
    state.query = payloadQuery;
  }

  state.items = payload.items || [];
  if (typeof payload.selected_index === "number") {
    state.selectedIndex = Math.max(0, Math.floor(payload.selected_index));
  }
  if (queryChanged && shouldSyncInput) {
    state.selectedIndex = 0;
  }

  return { shouldSyncInput, inputValue: payloadQuery, modeChanged };
}
