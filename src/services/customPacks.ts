/** Custom (player-assembled) modpacks — see `commands/custom_packs.rs`. */

import { call } from "./ipc";
import type { CustomPack, CustomPackItem, ModSource } from "@/types/modpack";

export const customPackService = {
  list: () => call<CustomPack[]>("custom_pack_list"),

  items: (packId: string) => call<CustomPackItem[]>("custom_pack_items", { packId }),

  create: (name: string, description?: string) =>
    call<CustomPack>("custom_pack_create", {
      name,
      description: description?.trim() ? description.trim() : null,
      iconUrl: null,
    }),

  /** Delete a pack. Instances created from it are untouched. */
  remove: (packId: string) => call<null>("custom_pack_delete", { id: packId }),

  /** Returns the refreshed item list so the UI never races the DB. */
  addItem: (packId: string, source: ModSource, projectId: string, versionId?: string) =>
    call<CustomPackItem[]>("custom_pack_add", {
      packId,
      source,
      projectId,
      versionId: versionId ?? null,
    }),

  removeItem: (packId: string, projectId: string) =>
    call<CustomPackItem[]>("custom_pack_remove", { packId, projectId }),

  /**
   * Set the game version + loader the pack resolves against
   * (`custom_pack_set_target` on the Rust side).
   */
  setTarget: (packId: string, gameVersion: string, loader?: string) =>
    call<CustomPack>("custom_pack_set_target", {
      packId,
      gameVersion,
      loader: loader ?? null,
    }),

  /**
   * Resolve the whole manifest and materialise it as a playable instance.
   * Returns the new `Instance`.
   */
  play: (packId: string) =>
    call<import("@/types/instance").Instance>("custom_pack_install", { packId }),
};

export type CustomPackService = typeof customPackService;
