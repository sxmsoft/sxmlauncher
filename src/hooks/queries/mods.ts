/** Modrinth / CurseForge search, install, and Java runtime provisioning. */

import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";

import { qk } from "@/lib/query-client";
import { modService } from "@/services";
import { toast } from "@/stores/ui";
import type { ModRequestRequest, ModSearchQuery, ModSource } from "@/types/modpack";

export function useModSearch(query: ModSearchQuery, enabled = true) {
  // An empty search box means "show me popular mods", so a missing or blank
  // query is still a valid request; the caller only opts out entirely.
  const hasQuery = query.query === undefined || query.query.trim().length > 0;

  return useQuery({
    queryKey: qk.modSearch(query),
    queryFn: () => modService.search(query),
    enabled: enabled && hasQuery,
    staleTime: 60_000,
    placeholderData: (previous) => previous,
  });
}

export function useModProject(id: string | null, source: ModSource) {
  return useQuery({
    queryKey: ["modProject", id, source],
    queryFn: () => modService.project(id!, source),
    enabled: id != null,
    staleTime: 5 * 60_000,
  });
}

/** Full version history of a project — the modpack detail view's list. */
export function useModAllVersions(id: string | null, source: ModSource, enabled = true) {
  return useQuery({
    queryKey: ["modAllVersions", id, source],
    queryFn: () => modService.allVersions(id!, source),
    enabled: enabled && id != null,
    staleTime: 5 * 60_000,
  });
}

export function useModVersions(
  id: string | null,
  source: ModSource,
  gameVersion: string,
  loader?: string,
) {
  return useQuery({
    queryKey: qk.modVersions(id ?? "none", source, gameVersion),
    queryFn: () => modService.versions(id!, source, gameVersion, loader),
    enabled: id != null && gameVersion.length > 0,
    staleTime: 5 * 60_000,
  });
}

export function useInstallMods(instanceId: string | null) {
  const client = useQueryClient();
  return useMutation({
    mutationFn: (requests: ModRequestRequest[]) => modService.install(instanceId!, requests),
    onSuccess: (plan) => {
      void client.invalidateQueries({ queryKey: qk.instanceMods(instanceId ?? "none") });
      void client.invalidateQueries({ queryKey: qk.instances });
      toast.success(
        `Installed ${plan.files.length} file${plan.files.length === 1 ? "" : "s"}`,
        `${(plan.totalBytes / 1_048_576).toFixed(1)} MiB · Java ${plan.javaMajor}`,
      );
    },
    onError: (error) => toast.error(error, "Mod install failed"),
  });
}

/** Single-mod download: the file (and its dependencies) land in the instance. */
export function useDownloadMod() {
  return useMutation({
    mutationFn: ({
      instanceId,
      source,
      projectId,
      versionId,
    }: {
      instanceId: string;
      source: ModSource;
      projectId: string;
      versionId?: string;
    }) => modService.download(instanceId, source, projectId, versionId),
    onSuccess: (relativePath) => {
      toast.success("Mod downloaded", relativePath);
    },
    onError: (error) => toast.error(error, "Download failed"),
  });
}

/** Create an instance from a `.mrpack` / CurseForge pack (creates its own instance). */
export function useInstallModpack() {
  const client = useQueryClient();
  return useMutation({
    mutationFn: ({
      projectId,
      versionId,
      name,
      source,
    }: {
      projectId: string;
      versionId?: string;
      name?: string;
      source?: import("@/types/modpack").ModSource;
    }) => modService.installModpack(projectId, versionId, name, source ?? "modrinth"),
    onSuccess: (instance) => {
      void client.invalidateQueries({ queryKey: qk.instances });
      toast.success(`Modpack installed as “${instance.name}”`, "Ready to launch");
    },
    onError: (error) => toast.error(error, "Modpack install failed"),
  });
}

export function useJavaRuntimes() {
  return useQuery({
    queryKey: qk.javaRuntimes,
    queryFn: modService.javaRuntimes,
    staleTime: 60_000,
  });
}

export function useInstallJava() {
  const client = useQueryClient();
  return useMutation({
    mutationFn: (major: number) => modService.javaInstall(major),
    onSuccess: (runtime) => {
      void client.invalidateQueries({ queryKey: qk.javaRuntimes });
      toast.success(`Java ${runtime.major} ready`, runtime.path);
    },
    onError: (error) => toast.error(error, "Could not provision Java"),
  });
}

/** Which JDK an instance (or game version) will launch with. */
export function useJavaResolve(
  gameVersion: string | null,
  loader?: string,
  instanceId?: string,
  enabled = true,
) {
  return useQuery({
    queryKey: ["javaResolve", gameVersion, loader ?? null, instanceId ?? null],
    queryFn: () => modService.javaResolve(gameVersion!, loader, instanceId),
    enabled: enabled && gameVersion != null && gameVersion.trim().length > 0,
    staleTime: 15_000,
  });
}

/** Probe a `java` executable the user picked by hand. */
export function useJavaProbe() {
  const client = useQueryClient();
  return useMutation({
    mutationFn: (path: string) => modService.javaProbe(path),
    onSuccess: (runtime) => {
      void client.invalidateQueries({ queryKey: qk.javaRuntimes });
      toast.success(`Found ${runtime.vendor} ${runtime.version}`, runtime.path);
    },
    onError: (error) => toast.error(error, "That is not a usable Java"),
  });
}

/** Download the JDK a game version needs. */
export function useInstallJavaForVersion() {
  const client = useQueryClient();
  return useMutation({
    mutationFn: ({ gameVersion, loader }: { gameVersion: string; loader?: string }) =>
      modService.javaInstallForVersion(gameVersion, loader),
    onSuccess: (runtime) => {
      void client.invalidateQueries({ queryKey: qk.javaRuntimes });
      toast.success(`Java ${runtime.major} ready`, runtime.path);
    },
    onError: (error) => toast.error(error, "Could not install the JDK"),
  });
}

/** Folder the launcher installs managed JDKs into. */
export function useJavaManagedRoot(enabled = false) {
  return useQuery({
    queryKey: ["javaManagedRoot"],
    queryFn: modService.javaManagedRoot,
    enabled,
    staleTime: Infinity,
  });
}
