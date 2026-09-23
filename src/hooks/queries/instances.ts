/** Instance queries, plus install/launch/kill actions. */

import { useEffect } from "react";

import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useTranslation } from "react-i18next";

import { qk } from "@/lib/query-client";
import { instanceService } from "@/services";
import { useUiStore, toast } from "@/stores/ui";
import type {
  CreateInstanceRequest,
  Instance,
  LaunchOptions,
  LaunchReport,
  UpdateInstanceRequest,
} from "@/types/instance";

export function useInstances() {
  const selected = useUiStore((state) => state.selectedInstanceId);
  const select = useUiStore((state) => state.selectInstance);

  const query = useQuery({ queryKey: qk.instances, queryFn: instanceService.list });

  // Keep a valid selection at all times: the Dashboard's Play card needs one.
  // A just-created id is written into the cache before this effect runs, so a
  // stale list must not snap the selection back to the previous first card.
  useEffect(() => {
    const instances = query.data;
    if (!instances || instances.length === 0) return;
    if (!selected) {
      select(instances[0]!.id);
      return;
    }
    if (instances.some((entry) => entry.id === selected)) return;
    if (!query.isFetching) {
      select(instances[0]!.id);
    }
  }, [query.data, query.isFetching, selected, select]);

  return query;
}

export function useInstance(id: string | null) {
  return useQuery({
    queryKey: qk.instance(id ?? "none"),
    queryFn: () => instanceService.get(id!),
    enabled: id != null,
  });
}

export function useSelectedInstance(): Instance | null {
  const { data } = useInstances();
  const selected = useUiStore((state) => state.selectedInstanceId);
  if (!data || data.length === 0) return null;
  if (!selected) return data[0] ?? null;
  return data.find((entry) => entry.id === selected) ?? null;
}

export function useInstanceMods(id: string | null) {
  return useQuery({
    queryKey: qk.instanceMods(id ?? "none"),
    queryFn: () => instanceService.mods(id!),
    enabled: id != null,
  });
}

/** Instances the backend currently has a live java process for. */
export function useRunningInstances() {
  return useQuery({
    queryKey: qk.running,
    queryFn: instanceService.running,
    staleTime: 5_000,
  });
}

export function useVersions(releasesOnly = true) {
  return useQuery({
    queryKey: [...qk.versions, releasesOnly],
    queryFn: () => instanceService.versionList(releasesOnly),
    staleTime: 30 * 60_000,
  });
}

const invalidateInstance = (client: ReturnType<typeof useQueryClient>, id?: string) => {
  void client.invalidateQueries({ queryKey: qk.instances });
  if (id) void client.invalidateQueries({ queryKey: qk.instance(id) });
};

export function useCreateInstance() {
  const client = useQueryClient();
  const select = useUiStore((state) => state.selectInstance);
  return useMutation({
    mutationFn: (request: CreateInstanceRequest) => instanceService.create(request),
    onSuccess: (instance) => {
      client.setQueryData<Instance[]>(qk.instances, (current) => {
        const list = current ?? [];
        if (list.some((entry) => entry.id === instance.id)) return list;
        return [instance, ...list];
      });
      select(instance.id);
      invalidateInstance(client, instance.id);
      toast.success(`Created “${instance.name}”`, "Install it from the Play page");
    },
    onError: (error) => toast.error(error, "Could not create the instance"),
  });
}

export function useUpdateInstance() {
  const client = useQueryClient();
  return useMutation({
    mutationFn: (request: UpdateInstanceRequest) => instanceService.update(request),
    onSuccess: (instance) => {
      invalidateInstance(client, instance.id);
      toast.success("Instance updated");
    },
    onError: (error) => toast.error(error, "Could not update the instance"),
  });
}

export function useDeleteInstance() {
  const client = useQueryClient();
  const select = useUiStore((state) => state.selectInstance);
  return useMutation({
    mutationFn: ({ id, deleteFiles }: { id: string; deleteFiles: boolean }) =>
      instanceService.remove(id, deleteFiles),
    onSuccess: (_result, variables) => {
      invalidateInstance(client, variables.id);
      select(null);
      toast.info("Instance removed", variables.deleteFiles ? "Files were deleted" : "Files were kept");
    },
    onError: (error) => toast.error(error, "Could not remove the instance"),
  });
}

export function useDuplicateInstance() {
  const client = useQueryClient();
  return useMutation({
    mutationFn: ({ id, newName }: { id: string; newName?: string }) =>
      instanceService.duplicate(id, newName),
    onSuccess: (instance) => {
      invalidateInstance(client, instance.id);
      toast.success(`Copied to “${instance.name}”`);
    },
    onError: (error) => toast.error(error, "Could not duplicate the instance"),
  });
}

export function useInstallInstance() {
  const client = useQueryClient();
  return useMutation({
    mutationFn: (id: string) => instanceService.install(id),
    onSuccess: (instance) => invalidateInstance(client, instance.id),
    onError: (error) => toast.error(error, "Install failed"),
  });
}

export function useToggleMod() {
  const client = useQueryClient();
  return useMutation({
    mutationFn: ({
      id,
      projectId,
      enabled,
    }: {
      id: string;
      projectId: string;
      enabled: boolean;
    }) => instanceService.toggleMod(id, projectId, enabled),
    onSuccess: (_result, variables) => {
      void client.invalidateQueries({ queryKey: qk.instanceMods(variables.id) });
      void client.invalidateQueries({ queryKey: qk.instance(variables.id) });
    },
    onError: (error) => toast.error(error, "Could not toggle the mod"),
  });
}

/**
 * Launch.
 *
 * The heavy lifting (install, verify, provision Java, open a bridge) happens in
 * the backend and reports through `job://progress`; this hook only reports the
 * outcome and, when the launch joined a session, registers it.
 */
export function useLaunchInstance() {
  const client = useQueryClient();
  const { t } = useTranslation();

  return useMutation({
    mutationFn: ({ id, options }: { id: string; options?: LaunchOptions }) =>
      instanceService.launch(id, options),
    onSuccess: (report: LaunchReport) => {
      void client.invalidateQueries({ queryKey: qk.instances });
      void client.invalidateQueries({ queryKey: qk.running });
      if (report.connectAddress) {
        toast.success("Launched into a session", `Game connected to ${report.connectAddress}`);
      } else {
        toast.success(`Launching ${report.instance.name}`, `pid ${report.pid}`);
      }
    },
    onError: (error) => toast.error(error, t("errors.couldNotLaunch")),
  });
}

export function useKillInstance() {
  const client = useQueryClient();
  return useMutation({
    mutationFn: (id: string) => instanceService.kill(id),
    onSuccess: (killed) => {
      void client.invalidateQueries({ queryKey: qk.running });
      if (killed) toast.info("Game stopped");
    },
    onError: (error) => toast.error(error, "Could not stop the game"),
  });
}

/** Open a folder picker and adopt the folder as an instance. */
export function useImportInstance() {
  const client = useQueryClient();
  return useMutation({
    mutationFn: async () => {
      const { open } = await import("@tauri-apps/plugin-dialog");
      const picked = await open({ directory: true, multiple: false, title: "Select a Minecraft folder" });
      if (typeof picked !== "string") return null;
      return instanceService.importFolder(picked);
    },
    onSuccess: (instance) => {
      if (!instance) return;
      invalidateInstance(client, instance.id);
      toast.success(`Imported “${instance.name}”`);
    },
    onError: (error) => toast.error(error, "Could not import that folder"),
  });
}
