import { describe, expect, it } from "vitest";

import { buildPresence, isPresenceWork, type PresenceCopy, type PresenceInput, type PresenceWork } from "./discord-presence";

const en: PresenceCopy = {
  home: "Browsing home",
  library: "Browsing the library",
  modpacks: "Browsing modpacks",
  profile: "Viewing profile",
  activity: "Viewing activity",
  settings: "In settings",
  multiplayer: "Preparing multiplayer",
  download: "Download in progress",
  launching: "Launching",
  hosting: "Preparing a P2P host",
  joining: "Preparing to join",
  hostingShort: "Hosting",
  multiplayerShort: "Multiplayer",
  brand: "SXMLAUNCHER",
};

const tr: PresenceCopy = {
  home: "Ana menüde geziniyor",
  library: "Kütüphaneye bakıyor",
  modpacks: "Modpaketlerine bakıyor",
  profile: "Profil ve hesaba bakıyor",
  activity: "Etkinliğe bakıyor",
  settings: "Ayarlarda",
  multiplayer: "Çok oyunculu hazırlığı",
  download: "İndirme durumu",
  launching: "Başlatılıyor",
  hosting: "P2P sunucu hazırlıyor",
  joining: "Sunucuya katılmaya hazırlanıyor",
  hostingShort: "Sunucu",
  multiplayerShort: "Çok oyunculu",
  brand: "SXMLAUNCHER",
};

function input(overrides: Partial<PresenceInput> = {}): PresenceInput {
  return {
    pathname: "/",
    copy: en,
    instances: [],
    runningIds: [],
    playingSinceMs: null,
    hostName: null,
    hostInstanceId: null,
    joinName: null,
    work: null,
    ...overrides,
  };
}

const fabric = {
  id: "fabric",
  name: "Fabric Survival",
  gameVersion: "1.21.1",
  loader: "fabric",
};

function work(overrides: Partial<PresenceWork>): PresenceWork {
  return {
    kind: "mod_download",
    stage: "downloading",
    label: "Sodium",
    finished: false,
    error: null,
    startedAtMs: 50,
    ...overrides,
  };
}

describe("discord presence", () => {
  it("describes each primary page in English and Turkish", () => {
    const pages: Array<[string, keyof PresenceCopy]> = [
      ["/", "home"],
      ["/library", "library"],
      ["/instances/abc", "library"],
      ["/modpacks", "modpacks"],
      ["/custom-packs", "modpacks"],
      ["/skin", "profile"],
      ["/activity", "activity"],
      ["/servers", "multiplayer"],
      ["/settings", "settings"],
    ];
    for (const [pathname, key] of pages) {
      expect(buildPresence(input({ pathname })).details).toBe(en[key]);
      expect(buildPresence(input({ pathname, copy: tr })).details).toBe(tr[key]);
      expect(buildPresence(input({ pathname })).state).toBe("SXMLAUNCHER");
      expect(buildPresence(input({ pathname })).startUnixMs).toBeNull();
    }
  });

  it("uses the exact Turkish home, library, and modpack lines", () => {
    expect(buildPresence(input({ copy: tr })).details).toBe("Ana menüde geziniyor");
    expect(buildPresence(input({ pathname: "/library", copy: tr })).details).toBe("Kütüphaneye bakıyor");
    expect(buildPresence(input({ pathname: "/modpacks", copy: tr })).details).toBe("Modpaketlerine bakıyor");
  });

  it("shows the running instance with loader and version", () => {
    const payload = buildPresence(
      input({
        pathname: "/library",
        instances: [fabric],
        runningIds: ["fabric"],
        playingSinceMs: 1_700_000_000_000,
      }),
    );
    expect(payload).toEqual({
      details: "Fabric Survival",
      state: "Fabric · 1.21.1",
      startUnixMs: 1_700_000_000_000,
    });
  });

  it("marks a hosted or joined game without dropping the version", () => {
    expect(
      buildPresence(
        input({
          instances: [fabric],
          runningIds: ["fabric"],
          hostName: "Fabric Survival",
          hostInstanceId: "fabric",
        }),
      ).state,
    ).toBe("Hosting · Fabric · 1.21.1");
    expect(
      buildPresence(
        input({
          copy: tr,
          instances: [fabric],
          runningIds: ["fabric"],
          joinName: "Friends",
        }),
      ).state,
    ).toBe("Çok oyunculu · Fabric · 1.21.1");
  });

  it("returns to the page once the game process is gone", () => {
    const payload = buildPresence(
      input({
        pathname: "/",
        copy: tr,
        instances: [fabric],
        runningIds: [],
      }),
    );
    expect(payload.details).toBe("Ana menüde geziniyor");
    expect(payload.startUnixMs).toBeNull();
  });

  it("prefers an active download over the page, and ignores finished jobs", () => {
    const active = buildPresence(
      input({
        pathname: "/library",
        work: work({ label: "Sodium" }),
      }),
    );
    expect(active.details).toBe("Download in progress");
    expect(active.state).toBe("Sodium");
    expect(active.startUnixMs).toBe(50);

    const idle = buildPresence(
      input({
        pathname: "/activity",
        copy: tr,
        work: work({ finished: true }),
      }),
    );
    expect(idle.details).toBe("Etkinliğe bakıyor");
  });

  it("describes host and join preparation when no game is running", () => {
    expect(
      buildPresence(input({ pathname: "/", hostName: "Skyblock", copy: tr })).details,
    ).toBe("P2P sunucu hazırlıyor");
    expect(buildPresence(input({ joinName: "Friends" })).details).toBe("Preparing to join");
    expect(buildPresence(input({ joinName: "Friends" })).state).toBe("Friends");
  });

  it("treats a live host row as steady, not a download", () => {
    expect(isPresenceWork(work({ kind: "p2p_host", stage: "running", label: "Hosting Skyblock" }))).toBe(
      false,
    );
    expect(isPresenceWork(work({ kind: "p2p_connect", stage: "connecting_p2p", label: "Join" }))).toBe(
      true,
    );
    expect(isPresenceWork(work({ kind: "launch", stage: "launching", label: "Fabric Survival" }))).toBe(
      true,
    );
  });
});
