/**
 * Barrel for the shared IPC types.
 *
 * These interfaces are the frontend half of the backend contract: every one of
 * them mirrors a `#[derive(Serialize)]` struct in `src-tauri/src/models` and is
 * kept in sync by hand. When you change a Rust model, change its twin here.
 */

export * from "./account";
export * from "./instance";
export * from "./modpack";
export * from "./server";
export * from "./system";

/** Error shape thrown by every Tauri command (`AppError` in Rust). */
export interface CommandError {
  code: string;
  message: string;
  /** True when retrying the same operation may plausibly succeed. */
  retryable: boolean;
}

/** Narrow an unknown thrown value into a `CommandError`. */
export function toCommandError(error: unknown): CommandError {
  if (typeof error === "object" && error !== null) {
    const candidate = error as Partial<CommandError>;
    if (typeof candidate.code === "string" && typeof candidate.message === "string") {
      return {
        code: candidate.code,
        message: candidate.message,
        retryable: candidate.retryable ?? false,
      };
    }
    if (error instanceof Error) {
      return { code: "OTHER", message: error.message, retryable: false };
    }
  }
  return { code: "OTHER", message: String(error), retryable: false };
}

/** Error codes the UI reacts to specifically. */
export const ERROR_UNAUTHORIZED = "UNAUTHORIZED";
export const ERROR_DIRECTORY = "DIRECTORY";
export const ERROR_INSTANCE_NOT_FOUND = "INSTANCE_NOT_FOUND";
