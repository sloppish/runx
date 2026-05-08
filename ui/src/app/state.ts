import type { RenderItem, RenderPayload } from "../types";

export interface LauncherState {
  query: string;
  configError: string | null;
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
}

export function createState(): LauncherState {
  return { query: "", configError: null, items: [], selectedIndex: 0 };
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
  state.configError =
    typeof payload.config_error === "string" ? payload.config_error : null;
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

  return { shouldSyncInput, inputValue: payloadQuery };
}
