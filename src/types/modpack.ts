/**
 * Mod and modpack interfaces — mirrors the Modrinth/CurseForge models in
 * `src-tauri/src/models/modpack.rs`. Both registries are normalized to the same
 * shape, so the UI never branches on where a mod came from (except for badges).
 */

export type ModSource = "modrinth" | "curseforge";

export interface ModHashes {
  sha1: string | null;
  sha512: string | null;
  murmur2: number | null;
}

export interface ModProject {
  id: string;
  slug: string;
  title: string;
  description: string;
  source: ModSource;
  projectType: string;
  iconUrl: string | null;
  downloads: number;
  followers: number;
  categories: string[];
  gameVersions: string[];
  loaders: string[];
  license: string | null;
  updatedAt: string | null;
  clientSide: string | null;
  serverSide: string | null;
}

export type PackDependencyKind =
  | "required"
  | "optional"
  | "incompatible"
  | "embedded";

export interface ModDependency {
  kind: PackDependencyKind;
  projectId: string | null;
  versionId: string | null;
  fileName: string | null;
}

export interface ModVersion {
  id: string;
  projectId: string;
  name: string;
  versionNumber: string;
  versionType: string;
  source: ModSource;
  gameVersions: string[];
  loaders: string[];
  downloads: number;
  fileName: string;
  downloadUrl: string;
  hashes: ModHashes;
  fileSize: number;
  publishedAt: string | null;
  dependencies: ModDependency[];
}

export interface ModSearchHit {
  id: string;
  slug: string;
  title: string;
  description: string;
  source: ModSource;
  projectType: string;
  iconUrl: string | null;
  downloads: number;
  categories: string[];
  gameVersions: string[];
  loaders: string[];
  latestVersion: string | null;
}

export interface ModSearchQuery {
  query?: string;
  source: ModSource;
  projectType?: string;
  gameVersion?: string;
  loader?: string;
  categories?: string[];
  sort?: string;
  index?: number;
  limit?: number;
}

export interface ModSearchResults {
  hits: ModSearchHit[];
  total: number;
  offset: number;
  limit: number;
  source: ModSource;
}

export type ModInclusionReason =
  | "requested"
  | "required_dependency"
  | "optional_dependency"
  | "from_pack_manifest"
  | "override";

export interface ResolvedMod {
  projectId: string;
  versionId: string;
  title: string;
  fileName: string;
  url: string;
  sha1: string | null;
  size: number;
  /** Path relative to the instance root, e.g. `mods/sodium.jar`. */
  destination: string;
  source: ModSource;
  required: boolean;
  reason: ModInclusionReason;
}

export interface ModConflict {
  projectId: string;
  title: string;
  withProjectId: string;
  withTitle: string;
  reason: string;
}

export interface PackTarget {
  gameVersion: string;
  loader: ModLoader;
}

export interface ResolvedPackPlan {
  target: PackTarget;
  files: ResolvedMod[];
  removals: string[];
  conflicts: ModConflict[];
  totalBytes: number;
  javaMajor: number;
}

/** Request body for `mod_install`. */
export interface ModRequestRequest {
  source: ModSource;
  projectId: string;
  versionId?: string;
  required?: boolean;
}

/** Java runtime discovered on the machine or downloaded by the launcher. */
export interface JavaRuntime {
  path: string;
  major: number;
  version: string;
  vendor: string;
  isManaged: boolean;
  architecture: string;
}

/** Which JDK an instance (or raw game version) will actually launch with. */
export interface JavaResolution {
  requiredMajor: number;
  /** `null` when nothing compatible is installed. */
  selected: JavaRuntime | null;
  /** Every runtime detected on the machine. */
  candidates: JavaRuntime[];
  /** Human explanation, always filled in. */
  message: string;
  /** `true` when `selected` satisfies `requiredMajor`. */
  compatible: boolean;
}

/** Install progress as emitted on `job://progress`. */
export type JobStage =
  | "queued"
  | "resolving"
  | "downloading"
  | "verifying"
  | "extracting"
  | "linking"
  | "provisioning_java"
  | "launching"
  | "connecting_p2p"
  | "registering"
  | "running"
  | "done"
  | "failed";

/** Which subsystem owns a job (drives the icon and the wording in the UI). */
export type JobKind =
  | "instance_install"
  | "modpack_install"
  | "mod_download"
  | "java_runtime"
  | "asset_hydration"
  | "launch"
  | "p2p_connect"
  | "p2p_host";

export interface ProgressEvent {
  jobId: string;
  kind: JobKind;
  stage: JobStage;
  label: string;
  completedUnits: number;
  totalUnits: number;
  bytesPerSecond: number;
  currentItem: string | null;
  detail: string | null;
  finished: boolean;
  error: string | null;
  startedAtMs: number;
}

/** Human wording for each stage shown in the launch card. */
export const STAGE_LABEL: Record<JobStage, string> = {
  queued: "Queued",
  resolving: "Resolving files",
  downloading: "Downloading",
  verifying: "Verifying hashes",
  extracting: "Extracting",
  linking: "Linking files",
  provisioning_java: "Preparing Java",
  launching: "Launching",
  connecting_p2p: "Connecting",
  registering: "Publishing session",
  running: "Running",
  done: "Done",
  failed: "Failed",
};

import type { ModLoader } from "./instance";

/** A player-assembled pack (CurseForge-style "custom profile"). */
export interface CustomPack {
  id: string;
  name: string;
  description: string | null;
  iconUrl: string | null;
  /** Target the pack resolves against; null until the first mod pins it. */
  gameVersion: string | null;
  /** `vanilla`/`fabric`/`forge`/…, null until pinned. */
  loader: string | null;
  createdAt: string;
  updatedAt: string;
}

/** One pinned project inside a custom pack. */
export interface CustomPackItem {
  packId: string;
  source: ModSource;
  projectId: string;
  /** Empty string = resolve latest compatible at install time. */
  versionId: string;
  addedAt: string;
}

/** Fields `custom_pack_update` accepts; `null` clears an optional column. */
export interface CustomPackPatch {
  name?: string;
  description?: string | null;
  gameVersion?: string | null;
  loader?: string | null;
}
