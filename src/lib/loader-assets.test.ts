import { describe, expect, it } from "vitest";

import { loaderAssetKey, loaderBg, loaderIcon } from "./loader-assets";

describe("loader assets", () => {
  it("maps each known loader onto its pack files", () => {
    for (const key of ["vanilla", "fabric", "quilt", "forge", "neoforge", "modpack"] as const) {
      expect(loaderBg(key)).toBe(`/loaders/bg-${key}.png`);
      expect(loaderIcon(key, 64)).toBe(`/loaders/icon-${key}-64.png`);
      expect(loaderIcon(key, 256)).toBe(`/loaders/icon-${key}-256.png`);
    }
  });

  it("folds NeoForge spellings and pack aliases onto one key", () => {
    expect(loaderAssetKey("NeoForge")).toBe("neoforge");
    expect(loaderAssetKey("neo_forge")).toBe("neoforge");
    expect(loaderAssetKey("neo-forge")).toBe("neoforge");
    expect(loaderBg("pack")).toBe("/loaders/bg-modpack.png");
    expect(loaderIcon("modpack", 256)).toBe("/loaders/icon-modpack-256.png");
  });

  it("sends unknown loaders to the modpack plate", () => {
    expect(loaderAssetKey("")).toBe("modpack");
    expect(loaderAssetKey("paper")).toBe("modpack");
    expect(loaderBg(null)).toBe("/loaders/bg-modpack.png");
  });
});
