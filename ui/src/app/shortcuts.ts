import type { Shortcut } from "../types";

export function isCtrlNextShortcut(event: KeyboardEvent): boolean {
  return (
    event.key === "ArrowDown" ||
    (event.ctrlKey && !event.metaKey && !event.altKey && event.code === "KeyN")
  );
}

export function isCtrlPreviousShortcut(event: KeyboardEvent): boolean {
  return (
    event.key === "ArrowUp" ||
    (event.ctrlKey && !event.metaKey && !event.altKey && event.code === "KeyP")
  );
}

export function isCopyShortcut(event: KeyboardEvent): boolean {
  return (
    event.code === "KeyC" &&
    event.metaKey &&
    !event.ctrlKey &&
    !event.altKey &&
    !event.shiftKey
  );
}

export function isPasteShortcut(event: KeyboardEvent): boolean {
  return (
    event.code === "KeyV" &&
    event.metaKey &&
    !event.ctrlKey &&
    !event.altKey &&
    !event.shiftKey
  );
}

export function matchesShortcut(
  event: KeyboardEvent,
  shortcut: Shortcut | null,
): boolean {
  if (!shortcut) {
    return false;
  }

  const keyMatches = !shortcut.key || event.key === shortcut.key;
  const codeMatches = !shortcut.code || event.code === shortcut.code;
  return (
    keyMatches &&
    codeMatches &&
    !!event.altKey === !!shortcut.alt &&
    !!event.ctrlKey === !!shortcut.ctrl &&
    !!event.metaKey === !!shortcut.meta &&
    !!event.shiftKey === !!shortcut.shift
  );
}
