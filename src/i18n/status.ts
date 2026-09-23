/**
 * Activity copy is decided in English inside `activityPresentation` so the
 * healthy/error rules stay stable. The view layer translates the known lines.
 * A real error sentence is shown as the backend wrote it.
 */

import type { TFunction } from "i18next";

const STATUS_KEYS: Record<string, string> = {
  "Service Running": "activity.status.serviceRunning",
  Listening: "activity.status.listening",
  "Waiting for players": "activity.status.waiting",
  "Session published": "activity.status.published",
  Queued: "activity.status.queued",
  "Resolving files": "activity.status.resolving",
  Downloading: "activity.status.downloading",
  "Verifying hashes": "activity.status.verifying",
  Extracting: "activity.status.extracting",
  "Linking files": "activity.status.linking",
  "Preparing Java": "activity.status.java",
  Launching: "activity.status.launching",
  Connecting: "activity.status.connecting",
  "Publishing session": "activity.status.registering",
  Running: "activity.status.running",
  Done: "activity.status.done",
  "Completed download": "activity.status.completedDownload",
  Failed: "activity.status.failed",
  "Starting…": "activity.status.starting",
};

export function translateActivityStatus(status: string, t: TFunction): string {
  const key = STATUS_KEYS[status];
  return key ? t(key) : status;
}
