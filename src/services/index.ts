/**
 * Service layer barrel.
 *
 * Rule of the codebase: **components never call `invoke`**. They go through a
 * store or a hook, which calls one of these services, which calls `call()`.
 * That keeps command names, argument shapes and error handling in one thin,
 * greppable place per domain.
 */

export { systemService, type SystemService } from "./system";
export { accountService, type AccountService } from "./account";
export { jobsService, type CancelOutcome, type JobsService } from "./jobs";
export { instanceService, type InstanceService } from "./instances";
export { customPackService, type CustomPackService } from "./customPacks";
export { modService, modRequest, type ModService } from "./mods";
export {
  networkService,
  formatShareCode,
  isCompleteShareCode,
  type NetworkService,
} from "./network";
export { BROWSER_MODE, CommandFailure, call, errorMessage, isDirectoryOffline, isTauri, isUnauthorized } from "./ipc";
export {
  GAME_EVENT,
  PROGRESS_EVENT,
  SESSION_EVENT,
  onGameState,
  onProgress,
  onSession,
  onUpdateProgress,
} from "./events";
