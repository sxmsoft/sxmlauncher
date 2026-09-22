/**
 * Frameless-window controls.
 *
 * The shell is a custom titlebar (`decorations: false` in `tauri.conf.json`), so
 * minimize/maximize/close are app-level calls. Outside Tauri (browser preview)
 * every one of these resolves to `false` instead of throwing, so the buttons
 * stay visible but inert.
 */

import { isTauri } from "@/services/ipc";

/** Handle to the main window, or `null` in a plain browser. */
async function currentWindow() {
  if (!isTauri()) return null;
  const { getCurrentWindow } = await import("@tauri-apps/api/window");
  return getCurrentWindow();
}

export async function minimizeWindow(): Promise<void> {
  await (await currentWindow())?.minimize();
}

export async function toggleMaximizeWindow(): Promise<void> {
  await (await currentWindow())?.toggleMaximize();
}

export async function closeWindow(): Promise<void> {
  await (await currentWindow())?.close();
}

/** Whether the window is currently maximized (drives the titlebar icon). */
export async function isWindowMaximized(): Promise<boolean> {
  return (await (await currentWindow())?.isMaximized()) ?? false;
}

/** Open an external URL in the user's browser, never inside the webview. */
export async function openExternal(url: string): Promise<void> {
  if (!isTauri()) {
    window.open(url, "_blank", "noopener,noreferrer");
    return;
  }
  const { openUrl } = await import("@tauri-apps/plugin-opener");
  await openUrl(url);
}

/** Reveal a folder in the OS file manager (Settings → Storage). */
export async function revealPath(path: string): Promise<void> {
  if (!isTauri()) return;
  const { revealItemInDir } = await import("@tauri-apps/plugin-opener");
  await revealItemInDir(path);
}
