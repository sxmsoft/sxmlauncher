import { describe, expect, it } from "vitest";

import en from "./locales/en.json";
import tr from "./locales/tr.json";

function leaves(value: unknown, prefix = ""): string[] {
  if (value != null && typeof value === "object" && !Array.isArray(value)) {
    return Object.entries(value as Record<string, unknown>).flatMap(([key, child]) =>
      leaves(child, prefix ? `${prefix}.${key}` : key),
    );
  }
  return [prefix];
}

describe("i18n catalogs", () => {
  it("gives Turkish and English the same keys", () => {
    expect(leaves(tr).sort()).toEqual(leaves(en).sort());
  });

  it("keeps the product name out of translated chrome", () => {
    const blob = JSON.stringify({ tr, en });
    expect(blob.toLowerCase()).not.toContain("misty");
    expect(blob).not.toContain("SXM Deck");
  });
});
