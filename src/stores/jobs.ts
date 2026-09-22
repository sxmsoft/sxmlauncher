/**
 * Job progress.
 *
 * The backend emits `job://progress` for installs, modpack downloads, asset
 * hydration, Java provisioning, launches and both sides of a P2P session. The UI
 * never polls: this store is the only consumer of that stream, and every
 * progress bar in the app reads from it.
 *
 * Events are keyed by `jobId` and kept newest-first. A finished job is retained
 * so the launch card can show "Done" instead of snapping back to idle, and is
 * dropped when a newer job for the same subsystem starts.
 *
 * Cancelling is cooperative: `cancel(jobId)` flips a flag in the backend (the
 * download stops at its next checkpoint) and hides the job locally at once, so
 * the status strip never shows a job that is already gone from the user's point
 * of view — even if one last progress event is still in flight.
 */

import { create } from "zustand";

import { jobsService } from "@/services/jobs";
import { toast } from "@/stores/ui";
import type { JobKind, ProgressEvent } from "@/types/modpack";

interface JobsState {
  /** Newest first. */
  jobs: ProgressEvent[];
  /** Job ids the user hid/cancelled: never re-added by late events. */
  dismissed: string[];
  /** Job ids with a cancel request in flight to the backend. */
  cancelling: string[];
  /** Jobs that have not finished and have not failed. */
  activeJobs: () => ProgressEvent[];
  /** The most recently started job, running or not. */
  latest: () => ProgressEvent | null;
  /** The newest job of a given kind (e.g. the current `launch`). */
  latestOfKind: (kind: JobKind) => ProgressEvent | null;
  ingest: (event: ProgressEvent) => void;
  dismiss: (jobId: string) => void;
  /** Stop a running job in the backend and hide it. Safe for finished jobs too. */
  cancel: (jobId: string) => Promise<void>;
  clearFinished: () => void;
  reset: () => void;
}

const MAX_JOBS = 20;

export const useJobsStore = create<JobsState>((set, get) => ({
  jobs: [],
  dismissed: [],
  cancelling: [],

  activeJobs: () => get().jobs.filter((job) => !job.finished && !job.error),

  latest: () => get().jobs[0] ?? null,

  latestOfKind: (kind) => get().jobs.find((job) => job.kind === kind) ?? null,

  ingest: (event) =>
    set((state) => {
      // A dismissed job stays gone: late progress events from a cancelled job
      // would otherwise resurrect its bar in the status strip.
      if (state.dismissed.includes(event.jobId)) {
        // The one exception: a *new* event with the same id after a terminal
        // state means the id was reused; accept it back.
        const known = state.jobs.find((job) => job.jobId === event.jobId);
        if (known && (known.finished || known.error)) return state;
      }
      const others = state.jobs.filter((job) => job.jobId !== event.jobId);
      const next = [event, ...others];
      // A new job of the same kind supersedes the old one's leftovers.
      const trimmed = next.filter(
        (job, index) =>
          index === 0 ||
          job.kind !== event.kind ||
          (!job.finished && !job.error) ||
          index < 3,
      );
      return { jobs: trimmed.slice(0, MAX_JOBS) };
    }),

  dismiss: (jobId) =>
    set((state) => ({
      jobs: state.jobs.filter((job) => job.jobId !== jobId),
      dismissed: [jobId, ...state.dismissed].slice(0, 100),
    })),

  cancel: async (jobId) => {
    const job = get().jobs.find((entry) => entry.jobId === jobId);
    // Already gone: hide it and report success so the ✕ button feels instant.
    if (!job || job.finished || job.error) {
      get().dismiss(jobId);
      return;
    }
    set((state) => ({
      cancelling: state.cancelling.includes(jobId)
        ? state.cancelling
        : [...state.cancelling, jobId],
      dismissed: state.dismissed.includes(jobId)
        ? state.dismissed
        : [jobId, ...state.dismissed].slice(0, 100),
    }));
    try {
      const outcome = await jobsService.cancel(jobId);
      if (!outcome.cancelled) {
        // The backend finished the job between our click and the RPC — nothing
        // to stop, and the final event is already on its way.
      }
    } catch (error) {
      // The job keeps running server-side, but the user pressed ✕: keep it
      // hidden and say so instead of resurrecting a bar they dismissed.
      toast.warning("Could not stop that job yet", error instanceof Error ? error.message : String(error));
    } finally {
      set((state) => ({
        cancelling: state.cancelling.filter((id) => id !== jobId),
        jobs: state.jobs.filter((entry) => entry.jobId !== jobId),
      }));
    }
  },

  clearFinished: () =>
    set((state) => ({ jobs: state.jobs.filter((job) => !job.finished && !job.error) })),

  reset: () => set({ jobs: [], dismissed: [], cancelling: [] }),
}));

/**
 * Aggregate progress of every active job, for the small global indicator in the
 * status strip: `{ completed, total, active }`.
 */
export function aggregateProgress(jobs: ProgressEvent[]): {
  completed: number;
  total: number;
  percent: number;
} {
  let completed = 0;
  let total = 0;
  for (const job of jobs) {
    completed += job.completedUnits;
    total += job.totalUnits;
  }
  const percent = total > 0 ? Math.min(100, (completed / total) * 100) : 0;
  return { completed, total, percent };
}
