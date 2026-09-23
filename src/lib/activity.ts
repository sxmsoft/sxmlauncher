/**
 * How an Activity row should look.
 *
 * P2P and hosting emit one-shot `job://progress` events that stay "active"
 * (not finished, no error) after the service is already up — or after NAT/STUN
 * has already failed. Those must not render as a download bar.
 */

import { STAGE_LABEL, type ProgressEvent } from "@/types/modpack";

export type ActivityMode = "work" | "steady" | "error";

export interface ActivityPresentation {
  mode: ActivityMode;
  /** Line under the job title. */
  status: string;
  showProgress: boolean;
  /** 0–100 when the total is known. `null` means an indeterminate bar. */
  progress: number | null;
}

const FAILURE =
  /\b(fail(?:ed|ure)?|unreachable|error|could not|cannot|timed out|timeout|refused)\b/i;

/** Live service copy. Checked only after failures are ruled out. */
const STEADY_TEXT =
  /\b(hosting|listening|published|established|detected local|guest connected|connected via|waiting for players|relayed|server running|nat ok)\b/i;

const WORK_STAGES = new Set<ProgressEvent["stage"]>([
  "queued",
  "resolving",
  "downloading",
  "verifying",
  "extracting",
  "linking",
  "provisioning_java",
  "launching",
  "connecting_p2p",
]);

function blob(job: ProgressEvent): string {
  return [job.label, job.detail, job.currentItem, job.error].filter(Boolean).join(" ");
}

function fraction(job: ProgressEvent): number | null {
  if (job.totalUnits <= 0) return null;
  return Math.min(100, (job.completedUnits / job.totalUnits) * 100);
}

function isFailure(job: ProgressEvent, text: string): boolean {
  return job.stage === "failed" || Boolean(job.error) || FAILURE.test(text);
}

/** A hosting/P2P snapshot that is up (or idle), not a file transfer. */
function isSteady(job: ProgressEvent, text: string): boolean {
  const p2p = job.kind === "p2p_host" || job.kind === "p2p_connect";
  const transfer = job.bytesPerSecond > 0 || (job.totalUnits > 1 && WORK_STAGES.has(job.stage));
  if (transfer) return false;

  if (job.stage === "running" || job.stage === "registering" || job.stage === "done") return true;

  // "Hosting <world>" is emitted as `resolving` but it is the live host row.
  if (p2p && job.stage === "resolving" && /^hosting\b/i.test(job.label)) return true;

  if (p2p && STEADY_TEXT.test(text)) return true;

  return false;
}

function calmStatus(job: ProgressEvent, text: string): string {
  if (/minecraft server ready|\bdone \(/i.test(text)) return "Service Running";
  if (/reconnecting/i.test(text)) return "Reconnecting";
  if (/listening|detected local/i.test(text)) return "Listening";
  if (/waiting for players/i.test(text)) return "Waiting for players";
  if (/\bpublished\b/i.test(text)) return "Session published";
  if (job.stage === "done" && job.kind !== "p2p_host" && job.kind !== "p2p_connect") {
    return "Completed download";
  }
  if (job.stage === "running" || /^hosting\b/i.test(job.label) || job.stage === "registering") {
    return "Service Running";
  }
  return "Service Running";
}

function workStatus(job: ProgressEvent, text: string): string {
  if (/^starting\b|waiting for (minecraft|the world|the server|server\b)/i.test(text)) {
    return "Starting…";
  }
  return STAGE_LABEL[job.stage];
}

export function activityPresentation(job: ProgressEvent): ActivityPresentation {
  const text = blob(job);
  const progress = fraction(job);

  if (isFailure(job, text)) {
    const detail = job.error || job.detail || job.currentItem;
    const status = detail && detail !== job.label ? detail : job.label || "Failed";
    return { mode: "error", status, showProgress: false, progress: null };
  }

  if (isSteady(job, text)) {
    return { mode: "steady", status: calmStatus(job, text), showProgress: false, progress: null };
  }

  if (WORK_STAGES.has(job.stage) || job.bytesPerSecond > 0) {
    return { mode: "work", status: workStatus(job, text), showProgress: true, progress };
  }

  // Leftover P2P status events are not downloads.
  if (job.kind === "p2p_host" || job.kind === "p2p_connect") {
    return { mode: "steady", status: calmStatus(job, text), showProgress: false, progress: null };
  }

  return { mode: "work", status: STAGE_LABEL[job.stage], showProgress: true, progress };
}

/**
 * The status strip's one line.
 *
 * Finished downloads stay in the job list so Activity can show "done", but they
 * are not live work. A completed authlib-injector fetch must not keep the bar
 * on Downloading after the game is already up.
 */
/**
 * P2P emits a new job id per snapshot. Older unfinished rows (a server-jar
 * download, "Starting integrated host") must not keep Activity on Downloading
 * after a newer host or join snapshot exists.
 *
 * `jobs` is newest-first, matching the jobs store.
 */
export function coalesceActivityJobs(jobs: ProgressEvent[]): ProgressEvent[] {
  const newestLive = new Map<string, string>();
  const newest = new Map<string, ProgressEvent>();
  for (const job of jobs) {
    if (job.kind !== "p2p_host" && job.kind !== "p2p_connect") continue;
    if (!newest.has(job.kind)) newest.set(job.kind, job);
    if (job.finished || job.error) continue;
    if (!newestLive.has(job.kind)) newestLive.set(job.kind, job.jobId);
  }
  return jobs.filter((job) => {
    if (job.kind !== "p2p_host" && job.kind !== "p2p_connect") return true;
    const latest = newest.get(job.kind);
    // A failed join is the snapshot that matters. Older "Opening a direct
    // tunnel" / "Joining …" rows must not stay on Connecting after it.
    if (
      job.kind === "p2p_connect" &&
      latest &&
      (latest.finished || latest.error) &&
      latest.jobId !== job.jobId &&
      !job.finished &&
      !job.error
    ) {
      return false;
    }
    if (job.finished || job.error) return true;
    return newestLive.get(job.kind) === job.jobId;
  });
}

export function selectStatusJob(jobs: ProgressEvent[]): ProgressEvent | null {
  const live = coalesceActivityJobs(jobs).filter((job) => !job.finished && !job.error);
  return (
    live.find((job) => activityPresentation(job).mode === "work") ??
    live.find((job) => activityPresentation(job).mode === "steady") ??
    null
  );
}
