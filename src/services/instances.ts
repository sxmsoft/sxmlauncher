/**
 * Instance service — CRUD, install, launch, per-instance mod toggles.
 *
 * Everything here operates on an isolated environment: the backend resolves
 * `instances/<id>/` from the id, so the UI never passes a path.
 */

import { call } from "./ipc";
import type {
  CreateInstanceRequest,
  GameStateEvent,
  Instance,
  LaunchOptions,
  LaunchReport,
  ManifestEntry,
  UpdateInstanceRequest,
} from "@/types";
import type { InstalledModRow } from "@/types/instance";

export const instanceService = {
  list: () => call<Instance[]>("instance_list"),

  get: (id: string) => call<Instance>("instance_get", { id }),

  create: (request: CreateInstanceRequest) =>
    call<Instance>("instance_create", { request }),

  update: (request: UpdateInstanceRequest) =>
    call<Instance>("instance_update", { request }),

  /** `deleteFiles: false` keeps the folder (useful before an import). */
  remove: (id: string, deleteFiles: boolean) =>
    call<void>("instance_delete", { id, deleteFiles }),

  duplicate: (id: string, newName?: string) =>
    call<Instance>("instance_duplicate", { id, newName: newName ?? null }),

  /** Download + verify everything the instance needs. Idempotent. */
  install: (id: string) => call<Instance>("instance_install", { id }),

  /** Recompute status/size/mod count from disk. */
  refresh: (id: string) => call<Instance>("instance_refresh", { id }),

  mods: (id: string) => call<InstalledModRow[]>("instance_mods", { id }),

  toggleMod: (id: string, projectId: string, enabled: boolean) =>
    call<void>("instance_toggle_mod", { id, projectId, enabled }),

  running: () => call<GameStateEvent[]>("instance_running"),

  /**
   * Install if needed, then start the game. Pass `options.connect` to drop
   * straight into a P2P session's bridge address.
   */
  launch: (id: string, options?: LaunchOptions) =>
    call<LaunchReport>("instance_launch", { id, options: options ?? null }),

  kill: (id: string) => call<boolean>("instance_kill", { id }),

  /** Adopt an existing folder (or a previously exported instance). */
  importFolder: (folder: string) => call<Instance>("instance_import", { folder }),

  versionList: (releasesOnly = true) =>
    call<ManifestEntry[]>("version_list", { releasesOnly }),
};

export type InstanceService = typeof instanceService;
