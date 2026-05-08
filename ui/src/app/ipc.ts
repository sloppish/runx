import { LauncherMessage } from "../types";

export function send(payload: LauncherMessage): void {
  window.ipc.postMessage(JSON.stringify(payload));
}
