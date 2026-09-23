import { describe, expect, it } from "vitest";

import { resolveAccent } from "./appearance";

describe("resolveAccent", () => {
  it("defaults to galactic purple", () => {
    expect(resolveAccent(undefined).hex).toBe("#8b5cf6");
    expect(resolveAccent("nope").id).toBe("violet");
  });

  it("keeps a named chip", () => {
    expect(resolveAccent("cyan")).toEqual({ id: "cyan", hex: "#38bdf8" });
  });

  it("accepts a live hex from the color picker", () => {
    expect(resolveAccent("#22C55E")).toEqual({ id: "custom", hex: "#22c55e" });
  });
});
