import { describe, expect, it } from "vitest";

import { elySkinUrl, publicTextureUrl } from "./skins";
import type { AccountSummary } from "@/types/account";

function account(partial: Partial<AccountSummary>): AccountSummary {
  return {
    id: "1",
    provider: "ely_by",
    username: "ensxm",
    uuid: "f45b2206-a023-4803-9faa-0a5729865586",
    skin: { model: "classic", skinUrl: null, capeUrl: null },
    hasStoredCredentials: true,
    expiresAt: null,
    lastUsedAt: "2026-01-01T00:00:00.000Z",
    ...partial,
  };
}

describe("publicTextureUrl", () => {
  it("upgrades Ely.by http textures so the webview can paint them", () => {
    expect(publicTextureUrl("http://ely.by/storage/skins/abc.png")).toBe(
      "https://ely.by/storage/skins/abc.png",
    );
    expect(publicTextureUrl("http://skinsystem.ely.by/textures/uuid")).toBe(
      "https://skinsystem.ely.by/textures/uuid",
    );
    expect(publicTextureUrl("https://textures.minecraft.net/texture/abc")).toBe(
      "https://textures.minecraft.net/texture/abc",
    );
  });

  it("points an Ely.by profile with no stored texture at skinsystem", () => {
    expect(elySkinUrl(account({}))).toBe(
      "https://skinsystem.ely.by/textures/f45b2206a02348039faa0a5729865586",
    );
  });
});
