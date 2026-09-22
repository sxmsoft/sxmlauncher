/**
 * Wallpaper: applies the user's appearance settings to the app shell.
 *
 * Reads `settings_get` once and writes CSS variables + the background layer:
 * `:root` gets `--wall-*` values, and `.app-wallpaper` gains an image or video
 * child depending on `uiBackgroundKind`. Runs exactly once (in `AppShell`), so
 * changing pages never tears the media down.
 */

import { useEffect } from "react";

import { useQuery } from "@tanstack/react-query";

import { qk } from "@/lib/query-client";
import { systemService } from "@/services";

const ACCENTS: Record<string, { primary: string; ring: string }> = {
  violet: { primary: "oklch(0.62 0.21 295)", ring: "oklch(0.62 0.21 295 / 60%)" },
  purple: { primary: "oklch(0.60 0.24 315)", ring: "oklch(0.60 0.24 315 / 60%)" },
  fuchsia: { primary: "oklch(0.65 0.24 340)", ring: "oklch(0.65 0.24 340 / 60%)" },
  indigo: { primary: "oklch(0.60 0.19 270)", ring: "oklch(0.60 0.19 270 / 60%)" },
  cyan: { primary: "oklch(0.72 0.15 200)", ring: "oklch(0.72 0.15 200 / 60%)" },
  emerald: { primary: "oklch(0.72 0.17 160)", ring: "oklch(0.72 0.17 160 / 60%)" },
};

export function useWallpaper(): void {
  const { data: settings } = useQuery({
    queryKey: qk.settings,
    queryFn: systemService.settings,
    staleTime: 30_000,
  });

  useEffect(() => {
    const root = document.documentElement;
    const layer = document.querySelector<HTMLElement>(".app-wallpaper");
    if (!settings) return;

    const accent = ACCENTS[settings.uiAccent] ?? ACCENTS.violet!;
    root.style.setProperty("--primary", accent.primary);
    root.style.setProperty("--ring", accent.ring);
    root.classList.toggle("no-animations", !settings.uiAnimations || settings.reduceMotion);

    if (!layer) return;
    layer.innerHTML = "";
    layer.style.opacity = "1";

    if (settings.uiBackgroundKind === "aurora" || !settings.uiBackgroundPath) return;

    const src = toAssetUrl(settings.uiBackgroundPath);
    if (!src) return;

    const opacity = Math.min(1, Math.max(0, settings.uiBackgroundOpacity));
    const blur = Math.min(40, Math.max(0, settings.uiBackgroundBlur));

    if (settings.uiBackgroundKind === "video") {
      const video = document.createElement("video");
      video.src = src;
      video.autoplay = true;
      video.loop = true;
      video.muted = true;
      video.playsInline = true;
      video.style.cssText = `width:100%;height:100%;object-fit:cover;opacity:${opacity};filter:blur(${blur}px);transform:scale(1.03);`;
      layer.appendChild(video);
    } else {
      const wrapper = document.createElement("div");
      wrapper.style.cssText = `width:100%;height:100%;background-image:url("${src}");background-size:cover;background-position:center;opacity:${opacity};filter:blur(${blur}px);transform:scale(1.03);`;
      layer.appendChild(wrapper);
    }
  }, [settings]);
}

/** Turn an absolute file path into something the webview can load. */
function toAssetUrl(path: string): string | null {
  const trimmed = path.trim();
  if (!trimmed) return null;
  if (/^(https?:|data:|blob:|asset:)/i.test(trimmed)) return trimmed;
  try {
    // Tauri's asset protocol serves absolute paths when the app data / instance
    // folders are in scope; fall back to a file URL otherwise.
    const { convertFileSrc } = (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ as {
      convertFileSrc?: (path: string) => string;
    };
    if (typeof convertFileSrc === "function") return convertFileSrc(trimmed);
  } catch {
    // plain browser: fall through to the raw path below.
  }
  return `file://${trimmed.replace(/\\/g, "/")}`;
}