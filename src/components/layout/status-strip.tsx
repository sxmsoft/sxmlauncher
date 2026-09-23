import { useEffect, useState } from "react";

import { Gauge, Globe, Radio, X } from "lucide-react";

import { StatusDot } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { IndeterminateProgress, Progress } from "@/components/ui/progress";
import { useNetworkStatus, useRunningInstances } from "@/hooks/queries";
import { activityPresentation } from "@/lib/activity";
import { cn, formatBytes } from "@/lib/utils";
import { aggregateProgress, useJobsStore } from "@/stores/jobs";
import { useSessionsStore } from "@/stores/sessions";

/**
 * Bottom strip: what the app is doing right now.
 *
 * One line, always visible: the newest job with a live progress bar, how many
 * games are running, the directory, and how many peers are connected. When
 * nothing is happening it reports idle state rather than disappearing, so the
 * layout never shifts.
 *
 * The ✕ button *stops* a running job (cancelling the download in the backend),
 * not just hides its bar; that is why pressing ✕ in the Activity panel and
 * pressing ✕ here behave the same.
 */
export function StatusStrip() {
  const jobs = useJobsStore((state) => state.jobs);
  const cancelling = useJobsStore((state) => state.cancelling);
  const cancel = useJobsStore((state) => state.cancel);
  const dismiss = useJobsStore((state) => state.dismiss);
  const { data: status } = useNetworkStatus();
  const { data: running } = useRunningInstances();
  const hosts = useSessionsStore((state) => Object.keys(state.hosts).length);
  const guests = useSessionsStore((state) => Object.keys(state.guests).length);

  const active = jobs.filter((job) => !job.finished && !job.error);
  const current = jobs[0] ?? null;
  const aggregate = aggregateProgress(active);

  // Throughput is only meaningful while something is downloading.
  const [rate, setRate] = useState(0);
  useEffect(() => {
    setRate(active.reduce((sum, job) => sum + job.bytesPerSecond, 0));
  }, [active]);

  const view = current ? activityPresentation(current) : null;
  const downloadLike = view?.mode === "work" && (current?.bytesPerSecond ?? 0) > 0;
  const isActive = current != null && !current.finished && !current.error && view?.mode === "work";
  const isCancelling = current != null && cancelling.includes(current.jobId);

  return (
    <footer className="flex h-8 shrink-0 items-center gap-3 border-t border-[var(--border)] bg-[color-mix(in_srgb,var(--surface-1)_80%,transparent)] px-3 font-mono text-[11px]">
      {current ? (
        <div className="flex min-w-0 flex-1 items-center gap-2">
          {view?.mode === "error" || current.error ? (
            <StatusDot tone="destructive" />
          ) : view?.mode === "steady" || current.finished ? (
            <StatusDot tone="success" pulse={view?.mode === "steady" && !current.finished} />
          ) : (
            <StatusDot tone="primary" pulse />
          )}
          <span className="truncate font-medium">
            {isCancelling ? "Stopping" : `${view?.status ?? ""} · ${current.label}`}
          </span>
          <span className="text-muted-foreground hidden truncate sm:inline">
            {current.currentItem ?? current.detail ?? ""}
          </span>

          {view?.showProgress ? (
            <div className="ml-2 hidden w-44 shrink-0 md:block">
              {view.progress == null ? (
                <IndeterminateProgress className="h-1.5" />
              ) : (
                <Progress value={view.progress} className="h-1.5" />
              )}
            </div>
          ) : null}

          {downloadLike && rate > 0 ? (
            <span className="text-muted-foreground shrink-0 tabular-nums">
              {formatBytes(rate)}/s
            </span>
          ) : null}

          {active.length > 1 ? (
            <span className="text-muted-foreground shrink-0">
              +{active.length - 1} more
            </span>
          ) : null}

          {view?.showProgress ? (
            <span className="text-muted-foreground ml-auto shrink-0 tabular-nums">
              {view.progress != null
                ? `${Math.round(view.progress)}%`
                : `${Math.round(aggregate.percent)}%`}
            </span>
          ) : null}

          {isActive ? (
            <Button
              variant="ghost"
              size="icon-sm"
              className="shrink-0"
              onClick={() => void cancel(current.jobId)}
              disabled={isCancelling}
            >
              <X className="size-3" />
              <span className="sr-only">Stop this download</span>
            </Button>
          ) : (
            <Button
              variant="ghost"
              size="icon-sm"
              className="shrink-0"
              onClick={() => dismiss(current.jobId)}
            >
              <X className="size-3" />
              <span className="sr-only">Dismiss progress</span>
            </Button>
          )}
        </div>
      ) : (
        <div className="flex min-w-0 flex-1 items-center gap-2">
          <StatusDot tone="muted" />
          <span className="text-muted-foreground">idle</span>
        </div>
      )}

      <div className="flex shrink-0 items-center gap-3">
        <span className="text-muted-foreground flex items-center gap-1.5">
          <Radio className="size-3" />
          {hosts} hosting · {guests} joined
        </span>
        <span className={cn("flex items-center gap-1.5", running && running.length > 0 ? "text-[var(--success)]" : "text-muted-foreground")}>
          <Gauge className="size-3" />
          {running?.length ?? 0} running
        </span>
        <span className="text-muted-foreground flex items-center gap-1.5">
          <Globe className="size-3" />
          {status?.directoryConnected ? `${status.onlinePlayers} online` : `LAN · ${status?.lanWorlds ?? 0} nearby`}
        </span>
      </div>
    </footer>
  );
}
