/**
 * Job control — the reverse direction of the `job://progress` stream.
 *
 * Cancelling is cooperative: the backend stops at its next checkpoint, emits a
 * terminal "cancelled" event and throws `CANCELLED` wherever an awaited command
 * was driving the job.
 */

import { call } from "./ipc";

/** What a cancel request did. */
export interface CancelOutcome {
  /** `true` when the job existed and is now stopping. */
  cancelled: boolean;
}

export const jobsService = {
  /** Ask a running job to stop. `false` is "already gone", not an error. */
  cancel: (jobId: string) => call<CancelOutcome>("job_cancel", { jobId }),

  /** Job ids that can still be cancelled (diagnostics / reconnect). */
  active: () => call<string[]>("job_active"),
};

export type JobsService = typeof jobsService;