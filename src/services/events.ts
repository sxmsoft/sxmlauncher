/**
 * Backend → UI event streams.
 *
 * The backend never asks the UI to poll: long jobs report through
 * `job://progress`, sessions through `session://event` and game processes
 * through `game://state`. All three are plain Tauri events, so subscribing is
 * `listen()` and unsubscribing is the returned function.
 *
 * Event names must stay in sync with `src-tauri/src/state.rs`
 * (`PROGRESS_EVENT`, `SESSION_EVENT`, `GAME_EVENT`).
 */

import { listen, type UnlistenFn } from "@tauri-apps/api/event";

import type { GameStateEvent } from "@/types/instance";
import type { ProgressEvent } from "@/types/modpack";
import type { SessionEventPayload } from "@/types/server";
import type { UpdateDownloadProgress } from "@/types/system";
import { isTauri } from "./ipc";

export const PROGRESS_EVENT = "job://progress";
export const SESSION_EVENT = "session://event";
export const GAME_EVENT = "game://state";

/** Subscribe to install/download/launch progress. */
export function onProgress(handler: (event: ProgressEvent) => void): Promise<UnlistenFn> {
  return subscribe<ProgressEvent>(PROGRESS_EVENT, handler);
}

/** Subscribe to P2P session lifecycle events (guests, kicks, relay fallback). */
export function onSession(handler: (event: SessionEventPayload) => void): Promise<UnlistenFn> {
  return subscribe<SessionEventPayload>(SESSION_EVENT, handler);
}

/** Subscribe to game process state (started, exited, crashed). */
export function onGameState(handler: (event: GameStateEvent) => void): Promise<UnlistenFn> {
  return subscribe<GameStateEvent>(GAME_EVENT, handler);
}

/** Update download progress from the Tauri updater plugin. */
export function onUpdateProgress(
  handler: (event: UpdateDownloadProgress) => void,
): Promise<UnlistenFn> {
  return subscribe<UpdateDownloadProgress>("updater://progress", handler);
}

/** `listen()` that resolves to a no-op outside Tauri. */
async function subscribe<T>(name: string, handler: (payload: T) => void): Promise<UnlistenFn> {
  if (!isTauri()) return () => {};
  return listen<T>(name, (event) => handler(event.payload));
}
