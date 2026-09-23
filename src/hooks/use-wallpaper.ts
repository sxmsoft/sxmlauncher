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

import { applyAppearance } from "@/lib/appearance";
import { qk } from "@/lib/query-client";
import { applyWallpaperVideo } from "@/lib/wallpaper";
import { systemService } from "@/services";

export function useWallpaper(): void {
  const { data: settings } = useQuery({
    queryKey: qk.settings,
    queryFn: systemService.settings,
    staleTime: 30_000,
  });

  useEffect(() => {
    const layer = document.querySelector<HTMLElement>(".app-wallpaper");
    if (!settings) return;

    applyAppearance(settings);

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
      applyWallpaperVideo(video, src, opacity, blur);
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
    // Tauri 2: `convertFileSrc` lives on the internals object and returns
    // `http://asset.localhost/...`, which CSP `media-src` must allow.
    const internals = (window as unknown as { __TAURI_INTERNALS__?: { convertFileSrc?: (path: string, protocol?: string) => string } })
      .__TAURI_INTERNALS__;
    if (typeof internals?.convertFileSrc === "function") {
      return internals.convertFileSrc(trimmed, "asset");
    }
  } catch {
    // plain browser: fall through to the raw path below.
  }
  return null;
}