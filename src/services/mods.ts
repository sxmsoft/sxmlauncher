/**
 * Mod service — Modrinth v2 + CurseForge v1 behind one normalized shape.
 *
 * The UI never branches on the registry except to draw a badge: `mod_search`,
 * `mod_project` and `mod_versions` return the same structures for both.
 */

import { call } from "./ipc";
import type { JavaRuntime, ModProject, ModRequestRequest, ModSearchQuery, ModSearchResults, ModSource, ModVersion, ResolvedPackPlan } from "@/types/modpack";
import type { Instance } from "@/types/instance";

export const modService = {
  /** Search mods or modpacks on either registry. */
  search: (query: ModSearchQuery) => call<ModSearchResults>("mod_search", { query }),

  project: (id: string, source: ModSource) => call<ModProject>("mod_project", { id, source }),

  /** Versions of a project that match a game version + loader. */
  versions: (id: string, source: ModSource, gameVersion: string, loader?: string) =>
    call<ModVersion[]>("mod_versions", {
      id,
      source,
      gameVersion,
      loader: loader ?? null,
    }),

  /** Every published version of a project, unfiltered (detail views). */
  allVersions: (id: string, source: ModSource) =>
    call<ModVersion[]>("mod_all_versions", { id, source }),

  /**
   * Resolve then install mods (with dependencies) into an instance.
   * Rejects with a `ModResolution` error listing conflicts before writing.
   */
  install: (instanceId: string, requests: ModRequestRequest[]) =>
    call<ResolvedPackPlan>("mod_install", { instanceId, requests }),

  /** Download a `.mrpack` / CurseForge zip, install it and create the instance it needs. */
  installModpack: (
    projectId: string,
    versionId?: string,
    name?: string,
    source: ModSource = "modrinth",
  ) =>
    call<Instance>("modpack_install", {
      projectId,
      versionId: versionId ?? null,
      name: name ?? null,
      source,
    }),

  /**
   * Download a single file (with its dependencies) into an instance without
   * touching anything else — the "I just want this one mod" path.
   */
  download: (instanceId: string, source: ModSource, projectId: string, versionId?: string) =>
    call<string>("mod_download", {
      instanceId,
      source,
      projectId,
      versionId: versionId ?? null,
    }),

  javaRuntimes: () => call<JavaRuntime[]>("java_runtimes"),

  /** Download a managed Temurin JDK for a major version. */
  javaInstall: (major: number) => call<JavaRuntime>("java_install", { major }),

  /**
   * Which JDK an instance (or raw game version) will actually launch with.
   * Reports the chosen runtime, what is installed, and a human explanation.
   */
  javaResolve: (gameVersion: string, loader?: string, instanceId?: string) =>
    call<JavaResolution>("java_resolve", {
      gameVersion,
      loader: loader ?? null,
      instanceId: instanceId ?? null,
    }),

  /** Probe a `java` executable the user picked by hand. */
  javaProbe: (path: string) => call<JavaRuntime>("java_probe", { path }),

  /** Download the JDK a game version needs. */
  javaInstallForVersion: (gameVersion: string, loader?: string) =>
    call<JavaRuntime>("java_install_for_version", {
      gameVersion,
      loader: loader ?? null,
    }),

  /** Folder the launcher installs managed JDKs into. */
  javaManagedRoot: () => call<string>("java_managed_root"),
};

export type ModService = typeof modService;

/** Convenience builder for the single-mod install case. */
export function modRequest(
  source: ModSource,
  projectId: string,
  versionId?: string,
): ModRequestRequest {
  return versionId ? { source, projectId, versionId } : { source, projectId };
}

/** Java runtime resolution shape (mirror of backend `java_resolve` result). */
export type JavaResolution = import("@/types/modpack").JavaResolution;
