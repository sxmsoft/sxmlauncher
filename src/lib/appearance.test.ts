import { describe, expect, it } from "vitest";

import { markForAccent, markSrc, nearestAccent, resolveAccent } from "./appearance";

describe("resolveAccent", () => {
  it("defaults to galactic purple", () => {
    expect(resolveAccent(undefined)).toEqual({ id: "purple", hex: "#8b5cf6", mark: "purple" });
    expect(resolveAccent("nope").mark).toBe("purple");
  });

  it("maps each swatch preset onto its hex and png slot", () => {
    expect(resolveAccent("cyan")).toEqual({ id: "cyan", hex: "#38bdf8", mark: "cyan" });
    expect(resolveAccent("magenta").hex).toBe("#e879f9");
    expect(resolveAccent("emerald").hex).toBe("#10b981");
    expect(resolveAccent("amber").hex).toBe("#f59e0b");
    expect(resolveAccent("silver").hex).toBe("#d4d4d8");
  });

  it("keeps older saved ids on the matching swatch", () => {
    expect(markForAccent("violet")).toBe("purple");
    expect(markForAccent("fuchsia")).toBe("magenta");
  });

  it("uses the exact swatch when the picker hex matches a preset", () => {
    expect(resolveAccent("#E879F9")).toEqual({ id: "magenta", hex: "#e879f9", mark: "magenta" });
  });

  it("picks the nearest swatch for a free hex and still paints that hex", () => {
    expect(resolveAccent("#22C55E")).toEqual({ id: "custom", hex: "#22c55e", mark: "emerald" });
    expect(nearestAccent("#38BDF8")).toBe("cyan");
  });

  it("points sidebar and header at the 32 and 64 pngs", () => {
    expect(markSrc("purple", 32)).toBe("/brand/sxmlauncher-mark-purple-32.png");
    expect(markSrc("amber", 64)).toBe("/brand/sxmlauncher-mark-amber-64.png");
  });
});
