import { RenderItem } from "../types";
import { escapeHtml, escapeAttr } from "./text";
import { LauncherState } from "./state";
import { send } from "./ipc";

export interface DomElements {
  shell: HTMLElement;
  results: HTMLElement;
  configError: HTMLElement;
  input: HTMLInputElement;
  inputWrap: HTMLElement;
}

export function queryElements(): DomElements {
  const shell = document.querySelector("main") as HTMLElement;
  const results = document.getElementById("results") as HTMLElement;
  const configError = document.getElementById("config-error") as HTMLElement;
  const input = document.getElementById("query") as HTMLInputElement;
  const inputWrap = document.querySelector(".input-wrap") as HTMLElement;
  return { shell, results, configError, input, inputWrap };
}

export function renderConfigError(dom: DomElements, error: string): void {
  dom.inputWrap.classList.add("hidden");
  dom.results.classList.add("hidden");
  dom.configError.classList.remove("hidden");
  dom.configError.innerHTML = `
    <div class="config-error-title">Config reload failed</div>
    <div class="config-error-copy">${escapeHtml(error)}</div>
  `;
}

export function renderResults(dom: DomElements, state: LauncherState): void {
  dom.inputWrap.classList.remove("hidden");
  dom.results.classList.remove("hidden");
  dom.configError.classList.add("hidden");
  dom.results.innerHTML = "";

  for (const [index, item] of state.items.entries()) {
    const row = document.createElement("button");
    row.type = "button";
    row.className = "item" + (item.compact ? " compact" : "") + (index === state.selectedIndex ? " selected" : "");
    const badgeMarkup = item.compact
      ? ""
      : item.icon
        ? `<div class="badge has-icon"><img class="icon-image" src="${escapeAttr(item.icon)}" alt="" /></div>`
        : `<div class="badge">${item.badge || ""}</div>`;
    const subtitleText = item.subtitle || "";
    const subtitleMarkup = item.compact
      ? ""
      : `<div class="subtitle" title="${escapeAttr(subtitleText)}">${escapeHtml(subtitleText)}</div>`;
    row.innerHTML = `
      ${badgeMarkup}
      <div class="copy">
        <div class="title">${escapeHtml(item.title)}</div>
        ${subtitleMarkup}
      </div>
      <div class="accelerator">${item.accelerator ? escapeHtml(item.accelerator) : ""}</div>
    `;
    row.addEventListener("click", () => send({ type: "activate", index, all_windows: false }));
    dom.results.appendChild(row);
  }
}

export function syncSelection(dom: DomElements, selectedIndex: number, ensureVisible = true): void {
  Array.from(dom.results.children).forEach((child, index) => {
    child.classList.toggle("selected", index === selectedIndex);
  });

  if (!ensureVisible) {
    return;
  }

  const selected = dom.results.children[selectedIndex];
  if (selected) {
    selected.scrollIntoView({ block: "nearest" });
  }
}

export function measurePreferredHeight(dom: DomElements, rows: number): number {
  const host = document.createElement("div");
  host.style.position = "fixed";
  host.style.left = "-10000px";
  host.style.top = "0";
  host.style.visibility = "hidden";
  host.style.pointerEvents = "none";
  host.style.padding = window.getComputedStyle(document.body).padding;

  const shell = document.createElement("main");
  shell.className = dom.shell.className;
  shell.style.height = "auto";
  const width = Math.max(
    Math.ceil(dom.shell.getBoundingClientRect().width),
    document.documentElement.clientWidth,
    720,
  );
  shell.style.width = `${width}px`;
  shell.style.overflow = "visible";

  const label = document.createElement("div");
  label.className = "label";
  label.innerHTML = "<strong>Runx</strong>";

  const inputWrap = document.createElement("div");
  inputWrap.className = "input-wrap";
  inputWrap.innerHTML = '<input type="text" autocomplete="off" spellcheck="false" placeholder="" value="" />';

  const results = document.createElement("section");
  results.className = "results";
  results.style.flex = "none";
  results.style.minHeight = "auto";
  results.style.overflow = "visible";

  for (let i = 0; i < rows; i++) {
    const row = document.createElement("button");
    row.type = "button";
    row.className = "item";
    row.innerHTML = `
      <div class="badge">AP</div>
      <div class="copy">
        <div class="title">Measure Row</div>
        <div class="subtitle">Preferred launcher height</div>
      </div>
      <div class="accelerator">Return</div>
    `;
    results.appendChild(row);
  }

  shell.appendChild(label);
  shell.appendChild(inputWrap);
  shell.appendChild(results);
  host.appendChild(shell);
  document.body.appendChild(host);

  try {
    return Math.ceil(host.getBoundingClientRect().height);
  } finally {
    host.remove();
  }
}
