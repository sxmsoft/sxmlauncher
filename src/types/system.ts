/**
 * Application-level interfaces — mirrors `src-tauri/src/config.rs`
 * (`AppSettings`) and `src-tauri/src/commands/system.rs`.
 */

import type { MemorySettings } from "./instance";

/**
 * User settings. Field-for-field identical to the Rust `AppSettings` struct,
 * which serializes as camelCase. Send this whole object back to
 * `settings_update` — the backend sanitizes it before persisting, so the UI can
 * pass user input straight through.
 */
export interface AppSettings {
  // --- downloads -------------------------------------------------------
  maxConcurrentDownloads: number;
  reDownloadOnHashMismatch: boolean;
  enableRangeRequests: boolean;
  keepDownloadCache: boolean;

  // --- game defaults ---------------------------------------------------
  defaultMemory: MemorySettings;
  autoProvisionJava: boolean;
  preferSystemJava: boolean;
  javaExtraRoots: string[];

  // --- accounts / providers ----------------------------------------------
  msaClientId: string;
  elybyClientId: string;
  elybyClientSecret: string | null;
  elybyRedirectUri: string;

  // --- p2p / hosting ---------------------------------------------------
  redisUrl: string;
  /** Embedded directory broker (MQTT). Empty = legacy Redis mode. */
  mqttBroker: string;
  mqttPort: number;
  relayUrl: string | null;
  stunServers: string[];
  shareByDefault: boolean;
  exposeLanEndpoints: boolean;
  maxHostedPlayers: number;
  hostPassword: string | null;
  directoryEnabled: boolean;
  lanDiscovery: boolean;
  lanPort: number;
  lanAutoDetect: boolean;

  // --- ui --------------------------------------------------------------
  theme: string;
  accent: string;
  reduceMotion: boolean;
  minimizeToTrayOnLaunch: boolean;
  closeToTray: boolean;
  /** `aurora` | `image` | `video`. */
  uiBackgroundKind: string;
  /** Absolute path to a user-imported background image/video. */
  uiBackgroundPath: string | null;
  /** 0–1 layer opacity for the custom background. */
  uiBackgroundOpacity: number;
  /** Gaussian blur (px) applied to the custom background. */
  uiBackgroundBlur: number;
  /** Accent hue key: violet | purple | fuchsia | indigo | cyan | emerald. */
  uiAccent: string;
  uiAnimations: boolean;
  uiCompact: boolean;

  // --- misc ------------------------------------------------------------
  curseforgeApiKey: string | null;
  lastSelectedInstance: string | null;
  analyticsEnabled: boolean;
}

export interface AppInfo {
  name: string;
  version: string;
  tauriVersion: string;
  rustVersion: string;
  os: string;
  arch: string;
  vaultBackend: string;
  curseforgeConfigured: boolean;
  maxIconBytes: number;
}

export interface PathsReport {
  root: string;
  instances: string;
  shared: string;
  java: string;
  cache: string;
  downloads: string;
  logs: string;
  database: string;
}

export interface CacheStats {
  downloadBytes: number;
  metadataRows: number;
  instanceBytes: number;
}

export interface RedisProbe {
  ok: boolean;
  onlinePlayers: number;
  message: string | null;
}

export type ToastTone = "info" | "success" | "warning" | "error";

/** Settings tabs, used for deep links from other pages. */
export type SettingsSection =
  | "general"
  | "appearance"
  | "downloads"
  | "defaults"
  | "network"
  | "accounts"
  | "fixes"
  | "storage"
  | "updates"
  | "diagnostics";

/** Result of an update feed check (`updater_check`). */
export interface UpdateInfo {
  /** Semver of the release on the feed. */
  version: string;
  /** Release notes from the feed (may be empty). */
  notes: string;
  /** `true` when the feed version is newer than the running app. */
  updateAvailable: boolean;
  /** Semver of the running app, echoed for display. */
  currentVersion: string;
}

/** Progress shape of a running update download (`updater://progress`). */
export interface UpdateDownloadProgress {
  downloadedBytes: number;
  totalBytes: number | null;
  done: boolean;
}

/** Where the update check stands — drives the Updates section wording. */
export type UpdateCheckState =
  | "idle"
  | "checking"
  | "up-to-date"
  | "available"
  | "downloading"
  | "installing"
  | "error";

/** Clamp a memory value into the range the JVM tolerates. */
export function clampMemory(mb: number): number {
  return Math.min(65536, Math.max(512, Math.round(mb)));
}
