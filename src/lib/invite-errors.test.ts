import { describe, expect, it } from "vitest";
import { createInstance } from "i18next";

import en from "@/i18n/locales/en.json";
import tr from "@/i18n/locales/tr.json";
import { translateInviteMessage } from "./invite-errors";

describe("translateInviteMessage", () => {
  it("maps directory failures to Turkish and English", async () => {
    const i18n = createInstance();
    await i18n.init({ lng: "tr", resources: { tr: { translation: tr }, en: { translation: en } } });
    const raw =
      "directory/signaling error: INVITE_EXPIRED: that connection code has expired or the host is offline";
    expect(translateInviteMessage(raw, i18n.t.bind(i18n))).toContain("süresi doldu");
    await i18n.changeLanguage("en");
    expect(translateInviteMessage(raw, i18n.t.bind(i18n))).toContain("expired");
    expect(translateInviteMessage("something else", i18n.t.bind(i18n))).toBe("something else");
  });
});
