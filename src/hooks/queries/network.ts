/** Server browser, hosting and joining. */

import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useTranslation } from "react-i18next";

import { translateInviteMessage } from "@/lib/invite-errors";
import { qk } from "@/lib/query-client";
import { networkService } from "@/services";
import { useSessionsStore } from "@/stores/sessions";
import { toast } from "@/stores/ui";
import type { HostRequest, JoinRejection, ServerFilter } from "@/types/server";

export function useServerBrowse(filter: ServerFilter) {
  return useQuery({
    queryKey: qk.servers(filter),
    queryFn: () => networkService.browse(filter),
    staleTime: 10_000,
    // The directory is a live list; a stale card is worse than a spinner.
    refetchInterval: 30_000,
    placeholderData: (previous) => previous,
  });
}

export function useFavorites() {
  return useQuery({ queryKey: qk.favorites, queryFn: networkService.favorites });
}

export function useSetFavorite() {
  const client = useQueryClient();
  return useMutation({
    mutationFn: ({ id, favorite }: { id: string; favorite: boolean }) =>
      networkService.setFavorite(id, favorite),
    onSuccess: () => void client.invalidateQueries({ queryKey: qk.favorites }),
  });
}

export function useNetworkStatus() {
  return useQuery({
    queryKey: qk.networkStatus,
    queryFn: networkService.status,
    refetchInterval: 20_000,
  });
}

/** "Will hosting work from this network?" — STUN classification. */
export function useNatProbe() {
  return useQuery({
    queryKey: qk.natProbe,
    queryFn: networkService.natProbe,
    staleTime: 5 * 60_000,
  });
}

export function useSessionHistory() {
  return useQuery({
    queryKey: qk.sessionHistory,
    queryFn: () => networkService.sessionHistory(25),
    staleTime: 30_000,
  });
}

export function useServerPing() {
  return useMutation({
    mutationFn: (id: string) => networkService.ping(id),
  });
}

/**
 * Start hosting.
 *
 * The backend does the work — bind the bridge, punch NAT, publish to Redis,
 * schedule heartbeats. This hook only records the resulting session so the
 * instance card can show its share code and guest list.
 */
export function useHostWorld() {
  const client = useQueryClient();
  const setHost = useSessionsStore((state) => state.setHost);
  const { t } = useTranslation();

  return useMutation({
    mutationFn: (request: HostRequest) => networkService.hostStart(request),
    onSuccess: (status, variables) => {
      setHost(status, variables.instanceId ?? null);
      void client.invalidateQueries({ queryKey: qk.networkStatus });
      toast.success(
        status.mode === "direct_p2p" ? "World published" : "World published via relay",
        `Share code ${status.shareCode}`,
      );
      if (status.mode === "relay") {
        toast.warning("Direct connection unavailable", status.natAdvice);
      }
    },
    onError: (error) =>
      toast.error(
        translateInviteMessage(error instanceof Error ? error.message : String(error), t),
        t("invite.hostFailed"),
      ),
  });
}

export function useStopHost() {
  const client = useQueryClient();
  const removeHost = useSessionsStore((state) => state.removeHost);

  return useMutation({
    mutationFn: (id: string) => networkService.hostStop(id),
    onSuccess: (_result, id) => {
      removeHost(id);
      void client.invalidateQueries({ queryKey: qk.networkStatus });
      toast.info("Stopped hosting", "The listing was removed from the browser");
    },
    onError: (error) => toast.error(error, "Could not stop hosting"),
  });
}

export function useKickGuest() {
  const client = useQueryClient();
  return useMutation({
    mutationFn: ({
      id,
      peerId,
      reason,
    }: {
      id: string;
      peerId: string;
      reason?: JoinRejection;
    }) => networkService.hostKick(id, peerId, reason),
    onSuccess: () => void client.invalidateQueries({ queryKey: qk.networkStatus }),
    onError: (error) => toast.error(error, "Could not remove that player"),
  });
}

/** Join by share code (validated locally before the network round trip). */
export function useJoinCode() {
  const client = useQueryClient();
  const setGuest = useSessionsStore((state) => state.setGuest);
  const { t } = useTranslation();
  return useMutation({
    mutationFn: (code: string) => networkService.joinCode(code),
    onSuccess: (status) => {
      setGuest(status);
      void client.invalidateQueries({ queryKey: qk.networkStatus });
      toast.success(`Connected to ${status.serverName}`, `Bridge on ${status.localAddress}:${status.localPort}`);
    },
    onError: (error) =>
      toast.error(
        translateInviteMessage(error instanceof Error ? error.message : String(error), t),
        t("invite.failedTitle"),
      ),
  });
}

export function useJoinServer() {
  const client = useQueryClient();
  const setGuest = useSessionsStore((state) => state.setGuest);
  const { t } = useTranslation();
  return useMutation({
    mutationFn: (id: string) => networkService.joinServer(id),
    onSuccess: (status) => {
      setGuest(status);
      void client.invalidateQueries({ queryKey: qk.networkStatus });
      toast.success(`Connected to ${status.serverName}`, `Bridge on ${status.localAddress}:${status.localPort}`);
    },
    onError: (error) =>
      toast.error(
        translateInviteMessage(error instanceof Error ? error.message : String(error), t),
        t("invite.failedTitle"),
      ),
  });
}

export function useLeaveSession() {
  const client = useQueryClient();
  const removeGuest = useSessionsStore((state) => state.removeGuest);
  return useMutation({
    mutationFn: (id: string) => networkService.leave(id),
    onSuccess: (_result, id) => {
      removeGuest(id);
      void client.invalidateQueries({ queryKey: qk.networkStatus });
    },
  });
}

/**
 * Browse the local network (probe + collect) plus the live cache for polling.
 *
 * `refetchInterval` polls the cheap `lan_worlds` path; the full `lanBrowse`
 * (which fires a probe) runs on mount and on manual refresh.
 */
export function useLanBrowse(enabled = true) {
  const client = useQueryClient();
  const query = useQuery({
    queryKey: qk.lan,
    queryFn: () => networkService.lanBrowse(900),
    enabled,
    staleTime: 3_000,
    refetchInterval: 4_000,
    placeholderData: (previous) => previous,
  });

  const refresh = async () => {
    await client.invalidateQueries({ queryKey: qk.lan });
    await query.refetch();
  };

  return { ...query, refresh };
}

/** Live LAN worlds without firing a new probe (cheap polling / status pill). */
export function useLanWorlds(enabled = true) {
  return useQuery({
    queryKey: qk.lanWorlds,
    queryFn: networkService.lanWorlds,
    enabled,
    staleTime: 2_000,
    refetchInterval: 5_000,
  });
}

/** Announce a world on the local network. */
export function useLanHostStart() {
  const client = useQueryClient();
  return useMutation({
    mutationFn: (request: import("@/types/server").LanHostRequest) =>
      networkService.lanHostStart(request),
    onSuccess: (host) => {
      void client.invalidateQueries({ queryKey: qk.lan });
      void client.invalidateQueries({ queryKey: qk.lanWorlds });
      void client.invalidateQueries({ queryKey: qk.networkStatus });
      toast.success(
        "Hosting on your network",
        `Friends can join at ${host.address}:${host.port}`,
      );
    },
    onError: (error) => toast.error(error, "Could not host on the LAN"),
  });
}

/** Stop announcing a LAN world. */
export function useLanHostStop() {
  const client = useQueryClient();
  return useMutation({
    mutationFn: (id: string) => networkService.lanHostStop(id),
    onSuccess: () => {
      void client.invalidateQueries({ queryKey: qk.lan });
      void client.invalidateQueries({ queryKey: qk.lanWorlds });
      void client.invalidateQueries({ queryKey: qk.networkStatus });
      toast.info("Stopped hosting", "The world is no longer visible on the network");
    },
    onError: (error) => toast.error(error, "Could not stop hosting"),
  });
}

/** Announcement attached to an instance, if one is being hosted. */
export function useLanHostForInstance(instanceId: string | null | undefined) {
  return useQuery({
    queryKey: qk.lanHostForInstance(instanceId ?? "none"),
    queryFn: () => networkService.lanHostForInstance(instanceId!),
    enabled: instanceId != null,
    staleTime: 2_000,
    refetchInterval: 5_000,
  });
}
