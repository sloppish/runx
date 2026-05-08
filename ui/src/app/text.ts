export function escapeHtml(value: string): string {
  return value
    .replaceAll("&", "&amp;")
    .replaceAll("<", "&lt;")
    .replaceAll(">", "&gt;");
}

export function escapeAttr(value: string): string {
  return escapeHtml(value).replaceAll('"', "&quot;");
}

export function selectedInputText(input: HTMLInputElement): string {
  const start = input.selectionStart ?? input.value.length;
  const end = input.selectionEnd ?? input.value.length;
  if (start === end) {
    return "";
  }
  return input.value.slice(Math.min(start, end), Math.max(start, end));
}

export function replaceInputSelection(input: HTMLInputElement, text: string): string {
  const start = input.selectionStart ?? input.value.length;
  const end = input.selectionEnd ?? input.value.length;
  const head = input.value.slice(0, Math.min(start, end));
  const tail = input.value.slice(Math.max(start, end));
  const next = head + text + tail;
  const cursor = head.length + text.length;
  input.value = next;
  if (input.setSelectionRange) {
    input.setSelectionRange(cursor, cursor);
  }
  return next;
}
