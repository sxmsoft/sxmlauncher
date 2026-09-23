import { describe, expect, it } from "vitest";

import { applyWallpaperVideo, type WallpaperVideoElement } from "./wallpaper";

function fakeVideo(): WallpaperVideoElement & { attrs: Record<string, string> } {
  const attrs: Record<string, string> = {};
  return {
    src: "",
    muted: false,
    defaultMuted: false,
    loop: false,
    autoplay: false,
    playsInline: false,
    preload: "",
    style: { cssText: "" },
    attrs,
    setAttribute(name: string, value: string) {
      attrs[name] = value;
    },
    play: () => Promise.resolve(),
  };
}

describe("applyWallpaperVideo", () => {
  it("plays muted, looping, and cropped to the window", () => {
    const video = fakeVideo();
    applyWallpaperVideo(video, "http://asset.localhost/wallpapers/loop.mp4", 0.4, 2);
    expect(video.muted).toBe(true);
    expect(video.defaultMuted).toBe(true);
    expect(video.loop).toBe(true);
    expect(video.autoplay).toBe(true);
    expect(video.playsInline).toBe(true);
    expect(video.attrs).toMatchObject({
      muted: "",
      loop: "",
      autoplay: "",
      playsinline: "",
    });
    expect(video.style.cssText).toContain("object-fit:cover");
    expect(video.src).toBe("http://asset.localhost/wallpapers/loop.mp4");
  });
});
