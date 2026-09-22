/**
 * TanStack Query client.
 *
 * Server state (instances, listings, accounts, settings) lives here; only
 * client state that must survive re-renders outside React lives in Zustand.
 *
 * Retry policy is deliberate: `AppError.retryable` is the backend's own verdict
 * about whether trying again could work (a flaky download, a dropped directory
 * connection), so we honour it instead of retrying every error three times.
 */

import { QueryClient } from "@tanstack/react-query";

import { CommandFailure } from "@/services/ipc";

export const queryClient = new QueryClient({
  defaultOptions: {
    queries: {
      staleTime: 15_000,
      gcTime: 5 * 60_000,
      refetchOnWindowFocus: false,
      retry: (failureCount, error) => {
        if (error instanceof CommandFailure) {
          return error.retryable && failureCount < 2;
        }
        return failureCount < 1;
      },
    },
    mutations: {
      retry: false,
    },
  },
});

/** Query keys, colocated so invalidation never drifts from the fetch. */
export const qk = {
  appInfo: ["appInfo"] as const,
  appPaths: ["appPaths"] as const,
  settings: ["settings"] as const,
  accounts: ["accounts"] as const,
  activeAccount: ["accounts", "active"] as const,
  vaultBackend: ["vaultBackend"] as const,
  instances: ["instances"] as const,
  instance: (id: string) => ["instances", id] as const,
  instanceMods: (id: string) => ["instances", id, "mods"] as const,
  running: ["instances", "running"] as const,
  versions: ["versions"] as const,
  javaRuntimes: ["javaRuntimes"] as const,
  modSearch: (query: unknown) => ["modSearch", query] as const,
  customPacks: ["customPacks"] as const,
  customPack: (id: string) => ["customPacks", id] as const,
  modVersions: (id: string, source: string, gameVersion: string) =>
    ["modVersions", id, source, gameVersion] as const,
  servers: (filter: unknown) => ["servers", filter] as const,
  lan: ["servers", "lan"] as const,
  lanWorlds: ["servers", "lan-worlds"] as const,
  lanHostForInstance: (id: string) => ["servers", "lan-host", id] as const,
  favorites: ["servers", "favorites"] as const,
  networkStatus: ["networkStatus"] as const,
  sessionHistory: ["sessionHistory"] as const,
  natProbe: ["natProbe"] as const,
  cacheStats: ["cacheStats"] as const,
  updateCheck: ["updateCheck"] as const,
};
