/**
 * Wallpaper playback contract.
 *
 * A `<video>` that is not muted, looping, and allowed to autoplay stays on the
 * first frame (or never starts) under the webview autoplay policy. `object-fit`
 * has to be on the element itself: a parent utility does not crop the frames.
 */

export interface WallpaperVideoElement {
  src: string;
  muted: boolean;
  defaultMuted: boolean;
  loop: boolean;
  autoplay: boolean;
  playsInline: boolean;
  preload: string;
  style: { cssText: string };
  setAttribute(name: string, value: string): void;
  play?: () => Promise<void>;
}

export function applyWallpaperVideo(
  video: WallpaperVideoElement,
  src: string,
  opacity: number,
  blur: number,
): void {
  video.muted = true;
  video.defaultMuted = true;
  video.loop = true;
  video.autoplay = true;
  video.playsInline = true;
  video.preload = "auto";
  video.setAttribute("muted", "");
  video.setAttribute("loop", "");
  video.setAttribute("autoplay", "");
  video.setAttribute("playsinline", "");
  video.style.cssText = [
    "position:absolute",
    "inset:0",
    "width:100%",
    "height:100%",
    "object-fit:cover",
    `opacity:${opacity}`,
    `filter:blur(${blur}px)`,
    "transform:scale(1.04)",
  ].join(";");
  video.src = src;
  void video.play?.().catch(() => undefined);
}
