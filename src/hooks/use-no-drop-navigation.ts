import { useEffect } from "react";

/**
 * Stops the webview's default behaviour for dropped files: "navigate to the
 * file". With `dragDropEnabled: false` (needed so real HTML5 drop events reach
 * React), Chromium falls back to that default — a mis-aimed drop anywhere
 * outside a styled drop zone would otherwise replace the launcher UI with a
 * raw PNG. Pages that implement drop targets call `preventDefault` themselves;
 * this hook is the safety net for everywhere else.
 */
export function useNoDropNavigation() {
  useEffect(() => {
    const swallow = (event: DragEvent) => {
      if (Array.from(event.dataTransfer?.types ?? []).includes("Files")) {
        event.preventDefault();
        if (event.dataTransfer) event.dataTransfer.dropEffect = "none";
      }
    };
    // Capture phase: fires before any React handler, and still works if a
    // drop lands on an element without its own handler.
    window.addEventListener("dragover", swallow, { capture: true });
    window.addEventListener("drop", swallow, { capture: true });
    return () => {
      window.removeEventListener("dragover", swallow, { capture: true });
      window.removeEventListener("drop", swallow, { capture: true });
    };
  }, []);
}
