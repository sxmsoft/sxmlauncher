/** Custom pack hooks — list, items, CRUD and the install-into-instance flow. */

import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";

import i18n from "@/i18n";
import { qk } from "@/lib/query-client";
import { customPackService } from "@/services/customPacks";
import { toast } from "@/stores/ui";
import type { ModSource } from "@/types/modpack";

export function useCustomPacks() {
  return useQuery({
    queryKey: qk.customPacks,
    queryFn: customPackService.list,
    staleTime: 15_000,
  });
}

export function useCustomPackItems(packId: string | null) {
  return useQuery({
    queryKey: [...qk.customPack(packId ?? "none"), "items"],
    queryFn: () => customPackService.items(packId!),
    enabled: packId != null,
    staleTime: 15_000,
  });
}

/** Mutations below keep both the list and the pack's own keys fresh. */
function usePackInvalidator() {
  const client = useQueryClient();
  return (packId?: string) => {
    void client.invalidateQueries({ queryKey: qk.customPacks });
    if (packId) void client.invalidateQueries({ queryKey: qk.customPack(packId) });
  };
}

export function useCreateCustomPack() {
  const invalidate = usePackInvalidator();
  return useMutation({
    mutationFn: ({ name, description }: { name: string; description?: string }) =>
      customPackService.create(name, description),
    onSuccess: (pack) => {
      invalidate(pack.id);
      toast.success(`Pack "${pack.name}" created`, "Open it and pin mods from the browser.");
    },
    onError: (error) => toast.error(error, "Could not create pack"),
  });
}

export function useDeleteCustomPack() {
  const invalidate = usePackInvalidator();
  return useMutation({
    mutationFn: (packId: string) => customPackService.remove(packId),
    onSuccess: () => {
      invalidate();
      toast.info("Pack deleted", "Installed instances are untouched.");
    },
    onError: (error) => toast.error(error, "Could not delete pack"),
  });
}

export function useCustomPackAddItem(packId: string) {
  const invalidate = usePackInvalidator();
  return useMutation({
    mutationFn: ({
      source,
      projectId,
      versionId,
    }: {
      source: ModSource;
      projectId: string;
      versionId?: string;
    }) => customPackService.addItem(packId, source, projectId, versionId),
    onSuccess: () => invalidate(packId),
    onError: (error) => toast.error(error, "Could not add to pack"),
  });
}

export function useCustomPackRemoveItem(packId: string) {
  const invalidate = usePackInvalidator();
  return useMutation({
    mutationFn: (projectId: string) => customPackService.removeItem(packId, projectId),
    onSuccess: () => invalidate(packId),
    onError: (error) => toast.error(error, "Could not remove from pack"),
  });
}

export function useCustomPackSetTarget() {
  const invalidate = usePackInvalidator();
  return useMutation({
    mutationFn: ({ packId, gameVersion, loader }: { packId: string; gameVersion: string; loader?: string }) =>
      customPackService.setTarget(packId, gameVersion, loader),
    onSuccess: (pack) => invalidate(pack.id),
    onError: (error) => toast.error(error, "Could not set pack target"),
  });
}

/** Resolve + install the pack; resolves with the new `Instance` on success. */
export function usePlayCustomPack() {
  const client = useQueryClient();
  return useMutation({
    mutationFn: (packId: string) => customPackService.play(packId),
    onSuccess: (instance) => {
      void client.invalidateQueries({ queryKey: qk.instances });
      void client.invalidateQueries({ queryKey: qk.instance(instance.id) });
      toast.success(i18n.t("browse.toastPack", { name: instance.name }), i18n.t("browse.toastPackReady"));
    },
    onError: (error) => toast.error(error, "Pack install failed"),
  });
}
