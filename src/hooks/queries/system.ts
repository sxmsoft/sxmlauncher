/** System queries: app info, paths, settings, caches, logs, self-update. */

import { useEffect, useState } from "react";

import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";

import { qk } from "@/lib/query-client";
import { isTauri, onUpdateProgress, systemService } from "@/services";
import { toast } from "@/stores/ui";
import type {
  AppSettings,
  UpdateCheckState,
  UpdateDownloadProgress,
  UpdateInfo,
} from "@/types/system";

export function useAppInfo() {
  return useQuery({
    queryKey: qk.appInfo,
    queryFn: systemService.info,
    staleTime: Infinity,
  });
}

export function useAppPaths() {
  return useQuery({
    queryKey: qk.appPaths,
    queryFn: systemService.paths,
    staleTime: Infinity,
  });
}

export function useSettings() {
  return useQuery({
    queryKey: qk.settings,
    queryFn: systemService.settings,
    staleTime: 60_000,
  });
}

/**
 * Persist settings. The backend returns the sanitized object, which we write
 * straight into the cache — so a clamped memory value is visible immediately
 * instead of the form showing what the user typed.
 */
export function useSaveSettings() {
  const client = useQueryClient();
  return useMutation({
    mutationFn: (settings: AppSettings) => systemService.updateSettings(settings),
    onSuccess: (saved) => {
      client.setQueryData(qk.settings, saved);
      void client.invalidateQueries({ queryKey: qk.networkStatus });
    },
    onError: (error) => toast.error(error, "Could not save settings"),
  });
}

export function useTestRedis() {
  return useMutation({
    mutationFn: (url: string) => systemService.testRedis(url),
  });
}

export function useCacheStats(enabled = true) {
  return useQuery({
    queryKey: qk.cacheStats,
    queryFn: systemService.cacheStats,
    enabled,
    staleTime: 30_000,
  });
}

export function useClearCache() {
  const client = useQueryClient();
  return useMutation({
    mutationFn: ({ downloads, metadata }: { downloads: boolean; metadata: boolean }) =>
      systemService.clearCache(downloads, metadata),
    onSuccess: (stats) => {
      client.setQueryData(qk.cacheStats, stats);
      toast.success("Cache cleared");
    },
    onError: (error) => toast.error(error, "Could not clear the cache"),
  });
}

/** How often the launcher may ask the update feed: once per day. */
const UPDATE_CHECK_INTERVAL_MS = 24 * 60 * 60_000;
/** localStorage keys — the check bookkeeping survives restarts. */
const UPDATE_LAST_CHECK_KEY = "sxmlauncher.updateLastCheck";
const UPDATE_INFO_CACHE_KEY = "sxmlauncher.updateInfo";
const UPDATE_ANNOUNCED_KEY = "sxmlauncher.updateAnnounced";

function readLastCheck(): number | null {
  try {
    const value = Number(localStorage.getItem(UPDATE_LAST_CHECK_KEY));
    return Number.isFinite(value) && value > 0 ? value : null;
  } catch {
    return null; // storage unavailable (private mode, webview policy) — just check
  }
}

function stampLastCheck() {
  try {
    localStorage.setItem(UPDATE_LAST_CHECK_KEY, String(Date.now()));
  } catch {
    // Ignore — a missing stamp only means the next startup checks again.
  }
}

/** Last feed answer, cached so the titlebar badge survives restarts silently. */
function readCachedUpdate(): UpdateInfo | null {
  try {
    const raw = localStorage.getItem(UPDATE_INFO_CACHE_KEY);
    if (!raw) return null;
    const parsed = JSON.parse(raw) as UpdateInfo;
    return typeof parsed?.version === "string" ? parsed : null;
  } catch {
    return null;
  }
}

function cacheUpdate(info: UpdateInfo) {
  stampLastCheck();
  try {
    localStorage.setItem(UPDATE_INFO_CACHE_KEY, JSON.stringify(info));
    // Back on the latest version: forget the old announcement so the next
    // future release gets announced again.
    if (!info.updateAvailable) localStorage.removeItem(UPDATE_ANNOUNCED_KEY);
  } catch {
    // Non-fatal — worst case the toast repeats next session.
  }
}

function readAnnounced(): string | null {
  try {
    return localStorage.getItem(UPDATE_ANNOUNCED_KEY);
  } catch {
    return null;
  }
}

function markAnnounced(version: string) {
  try {
    localStorage.setItem(UPDATE_ANNOUNCED_KEY, version);
  } catch {
    // Non-fatal.
  }
}

/**
 * Update state machine: check → download → install.
 *
 * A check that finds nothing (or cannot reach the feed) resolves as data —
 * being offline must not look like a crash. Download progress arrives on
 * `updater://progress`; the install swaps the binary and restarts the app.
 *
 * `auto: true` additionally polls the feed once a day while the launcher is
 * open and toasts when a *new* version appears — silent otherwise, so an
 * unreachable feed or an up-to-date install never interrupts the user.
 */
export function useUpdateCheck(options: { auto?: boolean } = {}) {
  // Automatic (titlebar/shell) consumers only open the network gate when the
  // last check is older than a day — an app restarted five times a day hits
  // the feed once. The Settings page calls this without `auto` and always
  // fetches: opening that page is an explicit intent.
  const last = readLastCheck();
  const [gateOpen, setGateOpen] = useState(!options.auto);
  useEffect(() => {
    if (!options.auto) return;
    if (last == null || Date.now() - last >= UPDATE_CHECK_INTERVAL_MS) setGateOpen(true);
  }, [options.auto, last]);

  const query = useQuery({
    queryKey: qk.updateCheck,
    queryFn: async () => {
      const info = await systemService.checkUpdate();
      cacheUpdate(info);
      return info;
    },
    staleTime: 10 * 60_000,
    // The browser preview answers from the mock, so the badge is testable
    // without the real updater plugin.
    enabled: gateOpen,
    refetchInterval: options.auto && gateOpen ? UPDATE_CHECK_INTERVAL_MS : false,
    // While the gate is closed the badge still renders from the cached feed
    // answer — no request, no noise, but "update" stays visible.
    initialData: options.auto ? (readCachedUpdate() ?? undefined) : undefined,
    // Timestamp the seed with the *real* check time, so an aged cache is
    // born stale: the moment the gate opens, React Query refetches instead
    // of treating the restored data as fresh.
    initialDataUpdatedAt: options.auto ? (last ?? undefined) : undefined,
  });

  // Announce a found update exactly once per version — across restarts too,
  // via the localStorage marker. A still-pending update must not nag the
  // user on every launch; the titlebar badge carries the reminder instead.
  const { data } = query;
  useEffect(() => {
    if (!options.auto || !gateOpen || !data?.updateAvailable) return;
    if (readAnnounced() === data.version) return;
    markAnnounced(data.version);
    toast.info(`Version ${data.version} available`, "Install it from Settings → Updates");
  }, [options.auto, gateOpen, data]);

  return query;
}

/**
 * Mount-once startup scheduler: if the last feed check is older than a day
 * (or there is none — first launch), refresh the shared `updateCheck` cache
 * entry. Nothing is logged or toasted unless a new version is actually found
 * (`useUpdateCheck({ auto: true })` owns the one toast for that case).
 */
/**
 * Mount-once startup hook for the app shell: owns the automatic cadence
 * (feed at most once per day, silent unless a new release is found).
 */
export function useAutoUpdateCheck() {
  useUpdateCheck({ auto: true });
}

export function useUpdateInstaller() {
  const client = useQueryClient();
  const [state, setState] = useState<UpdateCheckState>("idle");
  const [progress, setProgress] = useState<UpdateDownloadProgress | null>(null);

  useEffect(() => {
    if (!isTauri()) return;
    let unlisten: (() => void) | undefined;
    let disposed = false;
    void onUpdateProgress((event) => {
      setProgress(event);
      if (event.done) setState("installing");
    }).then((fn) => {
      if (disposed) fn();
      else unlisten = fn;
    });
    return () => {
      disposed = true;
      unlisten?.();
    };
  }, []);

  const download = useMutation({
    mutationFn: systemService.downloadUpdate,
    onMutate: () => setState("downloading"),
    onSuccess: () => {
      setState("installing");
      toast.info("Update ready", "The launcher will restart to apply it");
    },
    onError: (error) => {
      setState("error");
      toast.error(error, "Could not download the update");
    },
  });

  const install = useMutation({
    mutationFn: systemService.installUpdate,
    onMutate: () => setState("installing"),
    onError: (error) => {
      setState("error");
      toast.error(error, "Could not apply the update");
    },
  });

  const check = useMutation({
    mutationFn: systemService.checkUpdate,
    onMutate: () => setState("checking"),
    onSuccess: (info) => {
      setState(info.updateAvailable ? "available" : "up-to-date");
      // Write the result straight into the shared cache (invalidating would
      // not refetch a gate-closed automatic query) and count it as a real
      // feed check, exactly like an automatic one.
      client.setQueryData(qk.updateCheck, info);
      cacheUpdate(info);
      if (info.updateAvailable) {
        markAnnounced(info.version);
        toast.info(`Version ${info.version} available`, "Install it from Settings → Updates");
      }
    },
    onError: (error) => {
      setState("error");
      toast.error(error, "Update check failed");
    },
  });

  return { state, setState, progress, check, download, install };
}

export function useLogTail(name: string | null, lines = 200) {
  return useQuery({
    queryKey: ["logTail", name, lines],
    queryFn: () => systemService.logTail(name!, lines),
    enabled: name != null,
    staleTime: 5_000,
  });
}
