/**
 * Read-only(ish) stand-in for the Rust backend, used **only** when the UI is
 * opened in a plain browser (`pnpm dev` without Tauri, design review, the
 * in-editor preview pane).
 *
 * It exists so the shell, the launch card and the server browser can be built
 * and reviewed without a Rust toolchain — not to emulate the launcher. Anything
 * that would touch the filesystem, the network or a game process returns sample
 * data and says so. In the real app this file is dead code: {@link mockResponse}
 * is only reached when `window.__TAURI_INTERNALS__` is missing.
 */

import type { AccountSummary } from "@/types/account";
import type { Instance } from "@/types/instance";
import type { CustomPack, CustomPackItem, ModSearchResults, JavaRuntime } from "@/types/modpack";
import {
  DEFAULT_HEARTBEAT_TTL_SECS,
  SERVER_LISTING_SCHEMA,
  type CachedServer,
  type HostStatus,
  type LanHost,
  type LanReport,
  type LanWorld,
  type NetworkStatus,
  type ServerListing,
  type ServerListingSummary,
  type SessionHistory,
} from "@/types/server";
import type { AppInfo, AppSettings, CacheStats, PathsReport, RedisProbe } from "@/types/system";

const now = () => new Date().toISOString();

function instance(overrides: Partial<Instance> & { id: string; name: string }): Instance {
  return {
    description: "",
    icon: null,
    gameVersion: "1.21.1",
    loader: { kind: "fabric", version: "0.16.9", build: null },
    java: { overridePath: null, preferredMajor: 21, autoDownload: true, jvmArgs: [] },
    memory: { minMb: 2048, maxMb: 6144 },
    resolution: { width: 1280, height: 720, fullscreen: false },
    gameArgs: [],
    sourcePack: null,
    createdAt: now(),
    updatedAt: now(),
    status: "ready",
    modCount: 0,
    lastPlayedAt: null,
    totalPlaytimeSecs: 0,
    launchCount: 0,
    sizeBytes: 412_000_000,
    requiredJavaMajor: 21,
    ...overrides,
  } as Instance;
}

let instances: Instance[] = [
  instance({
    id: "3f1c1a4e-1111-4a1e-9c11-000000000001",
    name: "Vanilla 1.21.1",
    description: "Clean survival",
    loader: { kind: "vanilla", version: null, build: null },
    modCount: 0,
    lastPlayedAt: new Date(Date.now() - 3 * 3600_000).toISOString(),
    totalPlaytimeSecs: 14 * 3600 + 22 * 60,
    launchCount: 23,
    status: "ready",
    memory: { minMb: 2048, maxMb: 8192 },
  }),
  instance({
    id: "3f1c1a4e-4444-4a1e-9c11-000000000004",
    name: "Fabric Survival",
    description: "Fabric + performance mods",
    loader: { kind: "fabric", version: "0.16.9", build: null },
    modCount: 6,
    lastPlayedAt: new Date(Date.now() - 26 * 3600_000).toISOString(),
    status: "ready",
  }),
  instance({
    id: "3f1c1a4e-2222-4a1e-9c11-000000000002",
    name: "Create: Above & Beyond",
    description: "Modpack 1.18.2 · 240 mods",
    gameVersion: "1.18.2",
    loader: { kind: "forge", version: null, build: "40.2.0" },
    modCount: 240,
    sizeBytes: 3_400_000_000,
    requiredJavaMajor: 17,
    java: { overridePath: null, preferredMajor: 17, autoDownload: true, jvmArgs: [] },
    sourcePack: {
      source: "curseforge",
      projectId: "297330",
      versionId: "4021",
      name: "Create: Above and Beyond",
      versionNumber: "1.3",
      iconUrl: null,
    },
    status: "not_installed",
    memory: { minMb: 4096, maxMb: 10240 },
  }),
  instance({
    id: "3f1c1a4e-5555-4a1e-9c11-000000000005",
    name: "Quilt Meadow",
    description: "Quilt 1.21.1",
    loader: { kind: "quilt", version: "0.26.4", build: null },
    modCount: 4,
    status: "ready",
  }),
  instance({
    id: "3f1c1a4e-3333-4a1e-9c11-000000000003",
    name: "Skyblock SMP",
    description: "Server instance kept in sync with the friends' world",
    loader: { kind: "neoforge", version: null, build: "21.1.72" },
    modCount: 48,
    status: "update_available",
    requiredJavaMajor: 21,
    sourcePack: {
      source: "modrinth",
      projectId: "fabulously-optimized",
      versionId: "6.4.0",
      name: "Fabulously Optimized",
      versionNumber: "6.4.0",
      iconUrl: null,
    },
  }),
];

const account: AccountSummary = {
  id: "7d444840-9dc0-11d1-b245-5ffdce74fad2",
  provider: "microsoft",
  username: "SteveBuilder",
  uuid: "7d444840-9dc0-11d1-b245-5ffdce74fad2",
  skin: {
    model: "classic",
    skinUrl: "https://textures.minecraft.net/texture/1a4af718455d4aab528e7a61f86fa25e6a369d1768dcb13f7df319a713eb810b",
    capeUrl: null,
  },
  hasStoredCredentials: true,
  expiresAt: new Date(Date.now() + 3600_000).toISOString(),
  lastUsedAt: now(),
};

const elyAccount: AccountSummary = {
  id: "f45b2206-a023-4803-9faa-0a5729865586",
  provider: "ely_by",
  username: "ensxm",
  uuid: "f45b2206-a023-4803-9faa-0a5729865586",
  skin: {
    model: "classic",
    skinUrl: "http://ely.by/storage/skins/c8f42eb2b7fdd92a2a8d7189a34cc9a2.png",
    capeUrl: null,
  },
  hasStoredCredentials: true,
  expiresAt: new Date(Date.now() + 3600_000).toISOString(),
  lastUsedAt: now(),
};

const offlineAccount: AccountSummary = {
  id: "0f0f0f0f-0f0f-0f0f-0f0f-0f0f0f0f0f0f",
  provider: "offline",
  username: "Player",
  uuid: "b50ad385-829d-3141-a216-7e7d7539ba7f",
  skin: { model: "classic", skinUrl: null, capeUrl: null },
  hasStoredCredentials: false,
  expiresAt: null,
  lastUsedAt: now(),
};

const settings: AppSettings = {
  maxConcurrentDownloads: 12,
  reDownloadOnHashMismatch: true,
  enableRangeRequests: true,
  keepDownloadCache: true,
  defaultMemory: { minMb: 2048, maxMb: 6144 },
  autoProvisionJava: true,
  preferSystemJava: true,
  javaExtraRoots: [],
  msaClientId: "00000000402b5328",
  elybyClientId: "sxmlauncher3",
  elybyClientSecret: null,
  elybyRedirectUri: "http://localhost:25564/elyby/callback",
  redisUrl: "redis://127.0.0.1:6379/0",
  mqttBroker: "broker.emqx.io",
  mqttPort: 8883,
  relayUrl: "wss://relay.sxmlauncher.dev",
  stunServers: ["stun.l.google.com:19302", "stun.cloudflare.com:3478"],
  shareByDefault: true,
  exposeLanEndpoints: true,
  maxHostedPlayers: 8,
  hostPassword: null,
  directoryEnabled: false,
  lanDiscovery: true,
  lanPort: 44511,
  lanAutoDetect: true,
  theme: "dark",
  accent: "purple",
  reduceMotion: false,
  minimizeToTrayOnLaunch: false,
  closeToTray: true,
  uiBackgroundKind: "aurora",
  uiBackgroundPath: null,
  uiBackgroundOpacity: 0.35,
  uiBackgroundBlur: 0,
  uiAccent: "purple",
  uiAnimations: true,
  uiCompact: false,
  curseforgeApiKey: null,
  lastSelectedInstance: instances[0]?.id ?? null,
  analyticsEnabled: false,
  discordApplicationId: "",
};

/**
 * Expand a browser summary into the full listing shape.
 *
 * `server_favorites` returns *cached listings* — what the real backend stores
 * in SQLite so favourites keep working offline — while the browser returns
 * summaries. The favourites tab rebuilds a summary from the cached listing, so
 * the mock must serve both shapes from one fixture or they drift apart.
 */
function fullListing(summary: ServerListingSummary): ServerListing {
  return {
    schemaVersion: SERVER_LISTING_SCHEMA,
    id: summary.id,
    name: summary.name,
    description: summary.description,
    motd: "Hosted with SXMLAUNCHER",
    iconBase64: summary.iconBase64,
    owner: {
      name: summary.ownerName,
      uuid: "00000000-0000-4000-8000-000000000000",
      provider: "offline",
    },
    gameVersion: summary.gameVersion,
    loader: summary.loader,
    loaderVersion: null,
    modpack: summary.modpackName
      ? {
          source: "modrinth",
          projectId: summary.id,
          versionId: "mock-version",
          name: summary.modpackName,
          versionNumber: "1.0.0",
        }
      : null,
    requiredModIds: [],
    players: summary.players,
    connection: {
      mode: summary.mode,
      peerId: summary.id,
      publicKey: "mock-public-key",
      endpoints: [{ kind: "public", addr: "203.0.113.42:51109", expiresAt: null }],
      relay:
        summary.mode === "relay"
          ? {
              url: "wss://relay.sxmlauncher.dev",
              roomToken: "mock-room",
              region: summary.region,
              certFingerprint: null,
            }
          : null,
      sessionToken: "mock-session-token",
      protocolVersion: 767,
    },
    region: summary.region,
    tags: summary.tags,
    whitelist: { enabled: false, allowedUuids: [] },
    passwordProtected: summary.passwordProtected,
    worldName: `${summary.name} — world`,
    createdAt: now(),
    heartbeatAt: summary.heartbeatAt,
    ttlSecs: DEFAULT_HEARTBEAT_TTL_SECS,
    worldPlaytimeSecs: 5400,
  };
}

const servers: ServerListingSummary[] = [
  {
    id: "aaaa1111-0000-4000-8000-000000000001",
    name: "Alex's Survival World",
    description: "Vanilla+ survival with friends. No griefing.",
    iconBase64: null,
    ownerName: "AlexWarden",
    gameVersion: "1.21.1",
    loader: "fabric",
    modpackName: null,
    players: { online: 3, max: 8 },
    mode: "direct_p2p",
    region: "eu-central",
    tags: ["survival", "vanilla+"],
    passwordProtected: false,
    heartbeatAt: now(),
    ageMs: 4200,
    pingMs: 31,
    versionMismatch: false,
  },
  {
    id: "aaaa1111-0000-4000-8000-000000000002",
    name: "Create Engineering Co.",
    description: "Collaborative Create build, invite only.",
    iconBase64: null,
    ownerName: "MiraTinker",
    gameVersion: "1.20.1",
    loader: "forge",
    modpackName: "Create: Above and Beyond",
    players: { online: 5, max: 5 },
    mode: "relay",
    region: "us-east",
    tags: ["create", "builders"],
    passwordProtected: true,
    heartbeatAt: now(),
    ageMs: 8100,
    pingMs: null,
    versionMismatch: true,
  },
  {
    id: "aaaa1111-0000-4000-8000-000000000003",
    name: "Sunday Skyblock",
    description: "Casual skyblock, resets monthly.",
    iconBase64: null,
    ownerName: "Kai",
    gameVersion: "1.21.1",
    loader: "quilt",
    modpackName: null,
    players: { online: 0, max: 6 },
    mode: "direct_p2p",
    region: "ap-south",
    tags: ["skyblock"],
    passwordProtected: false,
    heartbeatAt: new Date(Date.now() - 90_000).toISOString(),
    ageMs: 90_000,
    pingMs: 168,
    versionMismatch: false,
  },
];

const modSearch: ModSearchResults = {
  hits: [
    {
      id: "sodium",
      slug: "sodium",
      title: "Sodium",
      description: "Modern rendering engine that drastically improves performance.",
      source: "modrinth",
      projectType: "mod",
      iconUrl: null,
      downloads: 84_213_004,
      categories: ["optimization"],
      gameVersions: ["1.21.1"],
      loaders: ["fabric"],
      latestVersion: "0.6.5",
    },
    {
      id: "iris",
      slug: "iris",
      title: "Iris Shaders",
      description: "Shaderpack loader compatible with OptiFine shaders.",
      source: "modrinth",
      projectType: "mod",
      iconUrl: null,
      downloads: 39_004_221,
      categories: ["shaders"],
      gameVersions: ["1.21.1"],
      loaders: ["fabric"],
      latestVersion: "1.8.1",
    },
    {
      id: "fabulously-optimized",
      slug: "fabulously-optimized",
      title: "Fabulously Optimized",
      description: "A simple, optimised, vanilla-like modpack.",
      source: "modrinth",
      projectType: "modpack",
      iconUrl: null,
      downloads: 12_889_004,
      categories: ["optimization", "lightweight"],
      gameVersions: ["1.21.1"],
      loaders: ["fabric"],
      latestVersion: "6.4.0",
    },
  ],
  total: 57,
  offset: 0,
  limit: 20,
  source: "modrinth",
};

const javaRuntimes: JavaRuntime[] = [
  { path: "/usr/lib/jvm/temurin-21", major: 21, version: "21.0.5", vendor: "Eclipse Adoptium", isManaged: false, architecture: "x86_64" },
  { path: "/usr/lib/jvm/temurin-17", major: 17, version: "17.0.13", vendor: "Eclipse Adoptium", isManaged: false, architecture: "x86_64" },
  { path: "/home/user/.local/share/sxmlauncher/java/temurin-8", major: 8, version: "1.8.0_432", vendor: "Eclipse Adoptium", isManaged: true, architecture: "x86_64" },
];

const lanWorlds: LanWorld[] = [
  {
    id: "bbbb1111-0000-4000-8000-000000000001",
    name: "Kai's Creative Plot",
    motd: "Come build with us",
    host: "Kai",
    address: "192.168.1.42",
    port: 51234,
    gameVersion: "1.21.1",
    loader: "fabric",
    players: 2,
    maxPlayers: 8,
    passwordProtected: false,
    protocolVersion: 767,
    tags: ["creative", "lan"],
    instanceId: null,
    worldName: "creative",
    lastSeenSecs: 1.2,
  },
];

const versionList = [
  "1.21.4", "1.21.3", "1.21.1", "1.21", "1.20.6", "1.20.4", "1.20.1", "1.19.4", "1.18.2", "1.17.1", "1.16.5", "1.12.2", "1.8.9",
].map((id) => ({
  id,
  type: "release",
  url: "",
  time: "2024-08-08T10:00:00Z",
  releaseTime: "2024-08-08T10:00:00Z",
  sha1: null,
  complianceLevel: 1,
}));

/** Every mocked command. Returning `undefined` means "not available". */

// --- custom packs (stateful so the browser preview can drive the flow) ---
let packSeq = 0;
const customPacks: CustomPack[] = [];
const customPackItems: CustomPackItem[] = [];

export function mockResponse(command: string, args?: Record<string, unknown>): unknown {
  switch (command) {
    // --- system ---------------------------------------------------------
    case "settings_get":
      return settings;
    case "settings_update":
      return (args?.settings as AppSettings | undefined) ?? settings;
    case "discord_presence_set":
      return { enabled: false };
    case "discord_presence_clear":
      return null;
    case "settings_test_redis":
      return { ok: true, onlinePlayers: 1_284, message: "connected to the server directory" } satisfies RedisProbe;    case "updater_check": {
      // Browser preview has no updater; report up-to-date so the section
      // renders. Setting `window.__sxmlMockUpdate = "9.9.9"` simulates a
      // release on the feed — used to test the badge and startup toast.
      const mockVersion =
        (globalThis as { __sxmlMockUpdate?: string }).__sxmlMockUpdate ??
        (() => {
          try {
            return sessionStorage.getItem("sxml.mockUpdate");
          } catch {
            return null;
          }
        })();
      return {
        version: mockVersion ?? "0.1.0",
        notes: mockVersion ? "Simulated release for preview testing" : "",
        updateAvailable: Boolean(mockVersion),
        currentVersion: "0.1.0",
      };
    }
    case "updater_download":
    case "updater_install":
      return null;
    case "app_info":
      return {
        name: "SXMLAUNCHER",
        version: "0.1.0",
        tauriVersion: "2.x",
        rustVersion: "1.8x",
        os: "browser-preview",
        arch: "wasm",
        vaultBackend: "browser-preview",
        curseforgeConfigured: false,
        maxIconBytes: 65536,
      } satisfies AppInfo;
    case "app_paths":
      return {
        root: "~/.local/share/dev.sxmlauncher.app",
        instances: "~/.local/share/dev.sxmlauncher.app/instances",
        shared: "~/.local/share/dev.sxmlauncher.app/shared",
        java: "~/.local/share/dev.sxmlauncher.app/java",
        cache: "~/.local/share/dev.sxmlauncher.app/cache",
        downloads: "~/.local/share/dev.sxmlauncher.app/cache/downloads",
        logs: "~/.local/share/dev.sxmlauncher.app/logs",
        database: "~/.local/share/dev.sxmlauncher.app/sxmlauncher.db",
      } satisfies PathsReport;
    case "cache_stats":
      return { downloadBytes: 8_912_004_221, metadataRows: 4_120, instanceBytes: 12_400_000_000 } satisfies CacheStats;
    case "cache_clear":
      return { downloadBytes: 0, metadataRows: 0, instanceBytes: 12_400_000_000 } satisfies CacheStats;
    case "log_tail":
      return [
        "[09:41:02] [main/INFO]: Loading Minecraft 1.21.1 with Fabric Loader 0.16.9",
        "[09:41:03] [main/WARN]: Mod `sodium` requires `fabric-api`, found OK",
        "[09:41:05] [main/INFO]: P2P bridge listening on 127.0.0.1:51109",
      ];

    // --- accounts -------------------------------------------------------
    case "account_list":
      return [account, elyAccount, offlineAccount];
    case "account_active":
      return account;
    case "account_set_active":
      return account;
    case "account_vault_backend":
      return "browser-preview";
    case "account_login_offline":
      return { ...offlineAccount, username: String(args?.username ?? "Player") };
    case "account_refresh":
      return account;
    case "account_refresh_skin": {
      const target =
        args?.id === offlineAccount.id
          ? offlineAccount
          : args?.id === elyAccount.id
            ? elyAccount
            : account;
      return target.skin;
    }
    case "account_upload_skin": {
      const target = args?.id === offlineAccount.id ? offlineAccount : account;
      const model = args?.model === "slim" ? "slim" : "classic";
      if (target.provider === "offline") {
        throw new Error("offline profiles have no provider to upload a skin to");
      }
      if (target.provider === "ely_by") {
        // Mirror the real backend: validated locally, upload happens on the site.
        return {
          uploaded: false,
          skin: target.skin,
          message: `Ely.by accepts skin uploads on its website — continue at https://ely.by/u${target.username}/skin`,
          url: `https://ely.by/u${target.username}/skin?username=${target.username}`,
        };
      }
      // Persist into the mock's shared account so the refetch that follows
      // the upload returns the new model (the real backend stores it too).
      target.skin = { ...target.skin, model };
      return {
        uploaded: true,
        skin: target.skin,
        message: null,
        url: null,
      };
    }

    // --- instances ------------------------------------------------------
    case "instance_list":
    case "instance_running":
      return command === "instance_list" ? instances : [];
    case "instance_get": {
      const id = String(args?.id);
      return instances.find((entry) => entry.id === id) ?? instances[0];
    }
    case "instance_create":
      return instances[0];
    case "instance_update":
    case "instance_duplicate":
    case "instance_refresh":
    case "instance_install":
    case "instance_import":
      return instances[0];
    case "instance_mods":
      return [
        { instanceId: String(args?.id), source: "modrinth", projectId: "sodium", versionId: "abc", title: "Sodium", fileName: "sodium-fabric-0.6.5.jar", sha1: null, enabled: true, installedAt: now() },
        { instanceId: String(args?.id), source: "modrinth", projectId: "iris", versionId: "def", title: "Iris Shaders", fileName: "iris-1.8.1.jar", sha1: null, enabled: true, installedAt: now() },
        { instanceId: String(args?.id), source: "curseforge", projectId: "238222", title: "JEI", versionId: "ghi", fileName: "jei-1.21.1.jar", sha1: null, enabled: false, installedAt: now() },
      ];
    case "instance_launch":
      return {
        instance: instances[0],
        pid: 4242,
        connectAddress: args?.options ? "127.0.0.1:51109" : null,
        sessionId: null,
        commandPreview: "java -Xmx6G -cp ... net.fabricmc.loader.impl.launch.knot.KnotClient --username SteveBuilder",
      };
    case "version_list":
      return versionList;

    // --- custom packs ----------------------------------------------------
    case "custom_pack_list":
      return [...customPacks].sort((a, b) => b.updatedAt.localeCompare(a.updatedAt));
    case "custom_pack_get":
      return customPacks.find((pack) => pack.id === String(args?.packId)) ?? null;
    case "custom_pack_create": {
      packSeq += 1;
      const pack: CustomPack = {
        id: `pack-${packSeq}`,
        name: String(args?.name ?? "New pack"),
        description: (args?.description as string | null) ?? null,
        iconUrl: null,
        gameVersion: null,
        loader: null,
        createdAt: now(),
        updatedAt: now(),
      };
      customPacks.push(pack);
      return pack;
    }
    case "custom_pack_set_target": {
      const pack = customPacks.find((entry) => entry.id === String(args?.packId));
      if (!pack) throw new Error("custom pack not found");
      pack.gameVersion = String(args?.gameVersion ?? "1.21.1");
      pack.loader = String(args?.loader ?? "vanilla");
      pack.updatedAt = now();
      return pack;
    }
    case "custom_pack_delete": {
      const packId = String(args?.id);
      const index = customPacks.findIndex((entry) => entry.id === packId);
      if (index >= 0) customPacks.splice(index, 1);
      for (let i = customPackItems.length - 1; i >= 0; i -= 1) {
        if (customPackItems[i]?.packId === packId) customPackItems.splice(i, 1);
      }
      return null;
    }
    case "custom_pack_items":
      return customPackItems.filter((item) => item.packId === String(args?.packId));
    case "custom_pack_add": {
      const packId = String(args?.packId);
      const pack = customPacks.find((entry) => entry.id === packId);
      if (!pack) throw new Error("custom pack not found");
      customPackItems.push({
        packId,
        source: (args?.source as CustomPackItem["source"]) ?? "modrinth",
        projectId: String(args?.projectId),
        versionId: (args?.versionId as string | null) ?? "",
        addedAt: now(),
      });
      if (!pack.gameVersion) pack.gameVersion = "1.21.1";
      if (!pack.loader) pack.loader = "fabric";
      pack.updatedAt = now();
      return customPackItems.filter((item) => item.packId === packId);
    }
    case "custom_pack_remove": {
      const packId = String(args?.packId);
      const projectId = String(args?.projectId);
      for (let i = customPackItems.length - 1; i >= 0; i -= 1) {
        const item = customPackItems[i];
        if (item?.packId === packId && item.projectId === projectId) {
          customPackItems.splice(i, 1);
        }
      }
      return customPackItems.filter((item) => item.packId === packId);
    }
    case "custom_pack_install": {
      const pack = customPacks.find((entry) => entry.id === String(args?.packId));
      if (!pack) throw new Error("custom pack not found");
      if (!customPackItems.some((item) => item.packId === pack.id)) {
        throw new Error("this pack has no mods yet - add mods from the browser first");
      }
      packSeq += 1;
      return instance({
        id: `pack-instance-${packSeq}`,
        name: `${pack.name} (pack)`,
      });
    }
    case "mod_download":
      return "mods/sodium-0.6.5.jar";

    // --- mods -----------------------------------------------------------
    case "mod_search": {
      const kind = (args?.query as { projectType?: string } | undefined)?.projectType;
      const sourceArg = (args?.query as { source?: string } | undefined)?.source ?? "modrinth";
      const all = modSearch.hits.filter((hit) => hit.source === sourceArg || true);
      // Mirror the real backend: the browser asks for one kind at a time, so
      // the preview shows the same list shape the live app will.
      return {
        ...modSearch,
        source: sourceArg,
        hits: kind ? all.filter((hit) => hit.projectType === kind) : all,
      };
    }
    case "mod_all_versions":
      return [
        {
          id: "F3aVnQ1",
          projectId: String(args?.id),
          name: `${String(args?.id)} 1.0.2`,
          versionNumber: "1.0.2",
          versionType: "release",
          source: "modrinth",
          gameVersions: ["1.21.1", "1.21", "1.20.1"],
          loaders: ["fabric"],
          downloads: 4_120_004,
          fileName: `${String(args?.id)}-1.0.2.jar`,
          downloadUrl: "https://cdn.modrinth.com/data/x/versions/y/x.jar",
          hashes: { sha1: "9f2c1a", sha512: null, murmur2: null },
          fileSize: 812_004,
          publishedAt: now(),
          dependencies: [],
        },
      ];
    case "mod_versions": {
      const id = String(args?.id);
      return [
        {
          id: "F3aVnQ1",
          projectId: id,
          name: `${id} 1.0.2`,
          versionNumber: "1.0.2",
          versionType: "release",
          source: "modrinth",
          gameVersions: [String(args?.gameVersion ?? "1.21.1")],
          loaders: [String(args?.loader ?? "fabric")],
          downloads: 4_120_004,
          fileName: `${id}-1.0.2.jar`,
          downloadUrl: "https://cdn.modrinth.com/data/x/versions/y/x.jar",
          hashes: { sha1: "9f2c1a", sha512: null, murmur2: null },
          fileSize: 812_004,
          publishedAt: now(),
          dependencies: [],
        },
      ];
    }
    case "java_runtimes":
      return javaRuntimes;
    case "java_install":
      return javaRuntimes[0];
    case "java_resolve": {
      const gameVersion =
        typeof (args?.gameVersion as string | undefined) === "string"
          ? (args?.gameVersion as string)
          : null;
      const required =
        gameVersion != null && /^1\.(1[7-9]|2[0-9]|[3-9])/.test(gameVersion)
          ? 17
          : 8;
      const selected =
        javaRuntimes.find((runtime) => runtime.major === required) ??
        javaRuntimes[0] ??
        null;
      return {
        requiredMajor: required,
        selected,
        candidates: javaRuntimes,
        message:
          selected != null
            ? `${selected.vendor} ${selected.version} will be used.`
            : "No compatible JDK is installed.",
        compatible: selected != null,
      };
    }
    case "java_probe": {
      const path = String(args?.path ?? "");
      return {
        path,
        major: 21,
        version: "21.0.5",
        vendor: "Temurin",
        isManaged: false,
        architecture: "x86_64",
      };
    }
    case "java_install_for_version":
      return javaRuntimes[0];
    case "java_managed_root":
      return "/home/user/.local/share/sxmlauncher/java";
    case "job_cancel":
      return { cancelled: true };
    case "job_active":
      return [];

    // --- network --------------------------------------------------------
    case "network_status":
      return {
        directoryConnected: false,
        directoryUrl: settings.redisUrl,
        onlinePlayers: 0,
        activeHosts: 0,
        activeGuests: 0,
        relayConfigured: true,
        message: "LAN mode: local worlds work with no server.",
        lanEnabled: true,
        lanPort: 44511,
        lanWorlds: 1,
      } satisfies NetworkStatus;
    case "lan_browse":
      return {
        enabled: true,
        port: 44511,
        worlds: lanWorlds,
        hosting: [],
        hint: "Same Wi-Fi only — no port forwarding and no server needed.",
      } satisfies LanReport;
    case "lan_worlds":
      return lanWorlds;
    case "lan_host_start": {
      const request = (args?.request ?? {}) as Record<string, unknown>;
      const host: LanHost = {
        id: "cccc1111-0000-4000-8000-000000000001",
        name: String(request.name ?? "My world"),
        port: Number(request.port ?? 51234),
        address: "192.168.1.7",
        instanceId: (request.instanceId as string | null) ?? null,
        worldName: (request.worldName as string | null) ?? null,
        gameVersion: String(request.gameVersion ?? "1.21.1"),
        players: 1,
        maxPlayers: Number(request.maxPlayers ?? 8),
        autoDetected: false,
      };
      return host;
    }
    case "lan_host_stop":
      return true;
    case "lan_host_for_instance":
      return null;
    case "lan_address":
      return "192.168.1.7:44511";
    case "server_browse":
      return servers;
    case "server_favorites":
      // Cached favourites are full listings, not browser summaries — the page
      // derives the summary itself.
      return servers.slice(0, 2).map((summary) => ({
        listing: fullListing(summary),
        favorite: summary.id === servers[0]?.id,
        lastSeenAt: now(),
      })) satisfies CachedServer[];
    case "server_ping":
      return 42;
    case "nat_probe":
      return {
        behavior: "endpoint_independent",
        publicAddress: "203.0.113.42:51109",
        canHostDirect: true,
        message: "Endpoint-independent NAT: direct P2P hosting should work.",
      };
    case "session_history":
      return { joins: [], p2p: [], relayRatio: 0.18 } satisfies SessionHistory;
    case "host_start":
      return {
        id: "bbbb2222-0000-4000-8000-000000000001",
        shareCode: "SXM1-AEAI-RMQG-NTKC-WSQT-ELAX-3T6N-CYAB-FF6E-NQ",
        mode: "direct_p2p",
        natAdvice: "Endpoint-independent NAT: direct P2P hosting should work.",
        publicEndpoint: "203.0.113.42:51109",
        guests: [],
        summary: { ...servers[0]!, name: String((args?.request as { name?: string } | undefined)?.name ?? servers[0]!.name), players: { online: 0, max: 8 } },
      } satisfies HostStatus;
    case "host_status":
      return undefined;
    case "join_code":
    case "join_server":
      return {
        id: "cccc3333-0000-4000-8000-000000000001",
        serverName: servers[0]!.name,
        mode: "direct_p2p",
        localAddress: "127.0.0.1",
        localPort: 51120,
        remote: "203.0.113.42:51109",
        rttMs: 34,
      };
    case "connection_code":
      return "SXM1-AEAI-RMQG-NTKC-WSQT-ELAX-3T6N-CYAB-FF6E-NQ";
    case "local_rtt":
      return 5;

    default:
      return undefined;
  }
}
