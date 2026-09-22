/**
 * The single choke point between the UI and the Rust backend.
 *
 * Everything goes through {@link invoke} so that:
 *   * command names and argument shapes live in one typed place,
 *   * errors are normalized into `CommandError` (never a raw `unknown`),
 *   * the app still boots in a plain browser (Vite dev / UI preview) by
 *     delegating to the read-only mock backend in `./mockBackend`.
 *
 * Tauri v2 converts camelCase argument keys to the snake_case parameter names
 * the Rust commands declare, so always pass camelCase keys from here.
 */

import { invoke as tauriInvoke } from "@tauri-apps/api/core";

import { toCommandError, type CommandError } from "@/types";
import { mockResponse } from "./mockBackend";

/** True when running inside the Tauri webview rather than a plain browser. */
export function isTauri(): boolean {
  if (typeof window === "undefined") return false;
  return "__TAURI_INTERNALS__" in window;
}

/** True when the UI is showing mock data because there is no Rust backend. */
export const BROWSER_MODE = !isTauri();

/** Thrown for every failed command, so callers never handle raw values. */
export class CommandFailure extends Error {
  readonly code: string;
  readonly retryable: boolean;

  constructor(error: CommandError) {
    super(error.message);
    this.name = "CommandFailure";
    this.code = error.code;
    this.retryable = error.retryable;
  }

  static from(error: unknown): CommandFailure {
    return error instanceof CommandFailure ? error : new CommandFailure(toCommandError(error));
  }
}

/** Invoke a command, normalizing failures into {@link CommandFailure}. */
export async function call<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  if (!isTauri()) {
    const mocked = mockResponse(command, args);
    if (mocked === undefined) {
      throw new CommandFailure({
        code: "NO_BACKEND",
        message: `\`${command}\` is unavailable in browser preview mode. Run \`pnpm dev:app\` for the full application.`,
        retryable: false,
      });
    }
    return mocked as T;
  }

  try {
    return await tauriInvoke<T>(command, args);
  } catch (error) {
    throw CommandFailure.from(error);
  }
}

/** True when a failure is a "you are not signed in" error. */
export function isUnauthorized(error: unknown): boolean {
  return error instanceof CommandFailure && error.code === "UNAUTHORIZED";
}

/** True when the Redis directory is unreachable. */
export function isDirectoryOffline(error: unknown): boolean {
  return error instanceof CommandFailure && error.code === "DIRECTORY";
}

/** Message suitable for a toast, for any thrown value. */
export function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}
