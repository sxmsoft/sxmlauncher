/**
 * System service — app info, paths, settings, caches, logs.
 *
 * `settingsUpdate` takes the whole settings object: the backend sanitizes it
 * (clamping memory, download concurrency, ...) and rebuilds the managers that
 * depend on it, so partial patches are never sent.
 */

import { call } from "./ipc";
import type {
  AppInfo,
  AppSettings,
  CacheStats,
  PathsReport,
  RedisProbe,
  UpdateInfo,
} from "@/types/system";

export const systemService = {
  info: () => call<AppInfo>("app_info"),

  paths: () => call<PathsReport>("app_paths"),

  settings: () => call<AppSettings>("settings_get"),

  updateSettings: (settings: AppSettings) =>
    call<AppSettings>("settings_update", { settings }),

  /** Verify a Redis endpoint before the user commits to it. */
  testRedis: (url: string) => call<RedisProbe>("settings_test_redis", { url }),

  cacheStats: () => call<CacheStats>("cache_stats"),

  clearCache: (downloads = true, metadata = false) =>
    call<CacheStats>("cache_clear", { downloads, metadata }),

  logTail: (name: string, lines = 200) => call<string[]>("log_tail", { name, lines }),

  /** Ask the release feed whether a newer launcher exists (signature-verified feed). */
  checkUpdate: () => call<UpdateInfo>("updater_check"),

  /** Download (and verify) the update; report bytes on `updater://progress`. */
  downloadUpdate: () => call<void>("updater_download"),

  /** Swap in the downloaded update by restarting the launcher. */
  installUpdate: () => call<void>("updater_install"),
};

export type SystemService = typeof systemService;
