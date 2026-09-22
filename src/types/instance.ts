/**
 * Instance interfaces — mirrors `src-tauri/src/models/instance.rs`.
 *
 * Every instance is an isolated environment: its own `mods/`, `config/`,
 * `saves/` and `resourcepacks/`, so two instances never share a classpath.
 */

import type { ModSource } from "./modpack";

export type LoaderKind = "vanilla" | "fabric" | "quilt" | "forge" | "neoforge";

export type InstanceStatus =
  | "not_installed"
  | "installing"
  | "ready"
  | "running"
  | "corrupted"
  | "update_available";

export interface ModLoader {
  kind: LoaderKind;
  /** Fabric/Quilt loader version, or the Forge/NeoForge version. */
  version: string | null;
  build: string | null;
}

export interface JavaSettings {
  overridePath: string | null;
  /** Preferred major version; `null` derives it from the game version. */
  preferredMajor: number | null;
  autoDownload: boolean;
  jvmArgs: string[];
}

export interface MemorySettings {
  minMb: number;
  maxMb: number;
}

export interface ResolutionSettings {
  width: number;
  height: number;
  fullscreen: boolean;
}

/** Reference to the modpack an instance was created from. */
export interface ModpackRefSource {
  source: ModSource;
  projectId: string;
  versionId: string;
  name: string;
  versionNumber: string;
  iconUrl: string | null;
}

export interface InstanceConfig {
  id: string;
  name: string;
  description: string;
  /** Emoji or an absolute icon path. */
  icon: string | null;
  gameVersion: string;
  loader: ModLoader;
  java: JavaSettings;
  memory: MemorySettings;
  resolution: ResolutionSettings;
  gameArgs: string[];
  sourcePack: ModpackRefSource | null;
  createdAt: string;
  updatedAt: string;
}

/** Config plus derived state (status, counters, size). */
export interface Instance extends InstanceConfig {
  status: InstanceStatus;
  modCount: number;
  lastPlayedAt: string | null;
  totalPlaytimeSecs: number;
  launchCount: number;
  sizeBytes: number;
  /** Java major version this instance requires. */
  requiredJavaMajor: number;
}

export interface CreateInstanceRequest {
  name: string;
  description?: string;
  gameVersion: string;
  loader?: ModLoader;
  memory?: MemorySettings;
  icon?: string;
  installNow?: boolean;
}

export interface UpdateInstanceRequest {
  id: string;
  name?: string;
  description?: string;
  icon?: string;
  gameVersion?: string;
  loader?: ModLoader;
  java?: JavaSettings;
  memory?: MemorySettings;
  resolution?: ResolutionSettings;
  gameArgs?: string[];
}

/** Options passed to `instance_launch`. */
export interface LaunchOptions {
  /** `host:port` of a local P2P bridge to join straight into a world. */
  connect?: string;
  startHost?: boolean;
  quickPlayWorld?: string;
}

export interface LaunchReport {
  instance: Instance;
  pid: number;
  connectAddress: string | null;
  sessionId: string | null;
  /** Redacted command line, safe to display. */
  commandPreview: string;
}

export type GameState = "starting" | "running" | "exited" | "crashed";

export interface GameStateEvent {
  instanceId: string;
  state: GameState;
  pid: number | null;
  connectAddress: string | null;
  message: string | null;
}

/** One row of the installed-mod list. */
export interface InstalledModRow {
  instanceId: string;
  source: ModSource;
  projectId: string;
  versionId: string;
  title: string;
  fileName: string;
  sha1: string | null;
  enabled: boolean;
  installedAt: string;
}

/** A Minecraft release from the Mojang version manifest. */
export interface ManifestEntry {
  id: string;
  type: string;
  url: string;
  time: string;
  releaseTime?: string | null;
  sha1?: string | null;
  complianceLevel?: number | null;
}

export const STATUS_LABEL: Record<InstanceStatus, string> = {
  not_installed: "Not installed",
  installing: "Installing",
  ready: "Ready",
  running: "Playing",
  corrupted: "Needs repair",
  update_available: "Update available",
};

export const STATUS_TONE: Record<InstanceStatus, string> = {
  not_installed: "text-muted-foreground",
  installing: "text-[var(--warning)]",
  ready: "text-[var(--success)]",
  running: "text-[var(--primary)]",
  corrupted: "text-[var(--destructive)]",
  update_available: "text-[var(--warning)]",
};

/** Instances whose files are usable without an install pass. */
export function isPlayable(instance: Instance): boolean {
  return instance.status === "ready" || instance.status === "running";
}
