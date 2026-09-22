import { ArrowDownToLine, Ban, CheckCircle2, ListVideo, Square, X } from "lucide-react";

import { Badge, StatusDot } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { IndeterminateProgress, Progress } from "@/components/ui/progress";
import { useKillInstance, useRunningInstances } from "@/hooks/queries";
import { aggregateProgress, useJobsStore } from "@/stores/jobs";
import { useUiStore } from "@/stores/ui";
import { STAGE_LABEL, type JobKind, type ProgressEvent } from "@/types/modpack";

const KIND_LABEL: Record<JobKind, string> = {
  instance_install: "Instance",
  modpack_install: "Modpack",
  mod_download: "Mod",
  java_runtime: "Java",
  asset_hydration: "Assets",
  launch: "Launch",
  p2p_connect: "P2P",
  p2p_host: "Hosting",
};

/**
 * Activity panel: every running and recent job in one place.
 *
 * It is the Downloads/Running tab the launcher was missing: instead of a thin
 * bar in the status strip, each job gets its own row with stage, live item,
 * throughput and a stop button that actually cancels the download server-side.
 * Running games are listed underneath with a kill switch.
 */
export function ActivityPanel() {
  const open = useUiStore((state) => state.activityOpen);
  const setOpen = useUiStore((state) => state.setActivityOpen);
  const jobs = useJobsStore((state) => state.jobs);
  const dismiss = useJobsStore((state) => state.dismiss);
  const clearFinished = useJobsStore((state) => state.clearFinished);

  const active = jobs.filter((job) => !job.finished && !job.error);
  const done = jobs.filter((job) => job.finished || job.error);
  const aggregate = aggregateProgress(active);

  if (!open) return null;

  return (
    <div className="pointer-events-none absolute inset-0 z-40">
      <button
        aria-label="Close activity"
        className="pointer-events-auto absolute inset-0 bg-black/50 backdrop-blur-[2px]"
        onClick={() => setOpen(false)}
      />
      <aside className="glass-strong pointer-events-auto absolute top-0 right-0 flex h-full w-[26rem] max-w-[92vw] flex-col gap-3 rounded-l-2xl border-white/10 p-4 shadow-2xl">
        <div className="flex items-center gap-2">
          <ListVideo className="size-4 text-[var(--primary)]" />
          <h2 className="flex-1 text-sm font-semibold tracking-tight">Activity</h2>
          {active.length > 0 ? (
            <Badge variant="primary">
              {active.length} running
              {aggregate.total > 0 ? ` · ${Math.round(aggregate.percent)}%` : ""}
            </Badge>
          ) : (
            <Badge variant="outline">idle</Badge>
          )}
          <Button variant="ghost" size="icon-sm" onClick={() => setOpen(false)}>
            <X className="size-4" />
          </Button>
        </div>

        <div className="flex min-h-0 flex-1 flex-col gap-2 overflow-y-auto pr-1">
          {active.length === 0 ? (
            <div className="flex flex-col items-center gap-2 rounded-xl border border-white/8 bg-black/20 px-4 py-8 text-center">
              <ArrowDownToLine className="size-6 text-[var(--muted-foreground)]" />
              <p className="text-sm font-medium">Nothing downloading right now</p>
              <p className="text-muted-foreground text-xs leading-relaxed">
                Installs, modpacks, assets and JDK downloads all land here — with a stop
                button that really stops them.
              </p>
            </div>
          ) : (
            active.map((job) => <ActiveJobRow key={job.jobId} job={job} />)
          )}

          {done.length > 0 ? (
            <div className="mt-1 flex flex-col gap-1.5">
              <div className="flex items-center gap-2 px-1">
                <span className="text-muted-foreground text-[11px] font-medium">
                  Recent ({done.length})
                </span>
                <div className="flex-1" />
                <Button variant="ghost" size="sm" onClick={clearFinished}>
                  Clear
                </Button>
              </div>
              {done.slice(0, 8).map((job) => (
                <div
                  key={job.jobId}
                  className="flex items-center gap-2 rounded-lg border border-white/6 bg-black/15 px-3 py-1.5"
                >
                  <StatusDot tone={job.error ? "destructive" : "success"} />
                  <span className="text-muted-foreground min-w-0 flex-1 truncate text-[11px]">
                    {job.error && !job.error.includes("cancelled")
                      ? `${job.label} — ${job.error}`
                      : `${job.label} · ${job.error ? "stopped" : "done"}`}
                  </span>
                  <Button variant="ghost" size="icon-sm" onClick={() => dismiss(job.jobId)}>
                    <X className="size-3" />
                  </Button>
                </div>
              ))}
            </div>
          ) : null}

          <RunningGames />
        </div>
      </aside>
    </div>
  );
}

function ActiveJobRow({ job }: { job: ProgressEvent }) {
  const cancelling = useJobsStore((state) => state.cancelling);
  const cancel = useJobsStore((state) => state.cancel);
  const stopping = cancelling.includes(job.jobId);
  const fraction =
    job.totalUnits > 0 ? Math.min(100, (job.completedUnits / job.totalUnits) * 100) : null;

  return (
    <div className="flex flex-col gap-1.5 rounded-xl border border-white/8 bg-black/25 p-3">
      <div className="flex items-center gap-2">
        <StatusDot tone="primary" pulse />
        <span className="min-w-0 flex-1 truncate text-xs font-medium">{job.label}</span>
        <Badge variant="outline">{KIND_LABEL[job.kind]}</Badge>
        <Button
          variant="ghost"
          size="icon-sm"
          disabled={stopping}
          onClick={() => void cancel(job.jobId)}
          title="Stop this job"
        >
          <Ban className="size-3.5" />
        </Button>
      </div>
      <div className="text-muted-foreground truncate text-[11px]">
        {stopping ? "Stopping…" : STAGE_LABEL[job.stage]}
        {job.currentItem ? ` · ${job.currentItem}` : job.detail ? ` · ${job.detail}` : ""}
      </div>
      {fraction == null ? (
        <IndeterminateProgress className="h-1.5" />
      ) : (
        <div className="flex items-center gap-2">
          <Progress value={fraction} className="h-1.5 flex-1" />
          <span className="text-muted-foreground shrink-0 text-[10px] tabular-nums">
            {Math.round(fraction)}%
          </span>
        </div>
      )}
      {job.bytesPerSecond > 0 ? (
        <div className="text-muted-foreground text-[10px] tabular-nums">
          {(job.bytesPerSecond / 1_048_576).toFixed(1)} MiB/s
        </div>
      ) : null}
    </div>
  );
}

function RunningGames() {
  const { data: running } = useRunningInstances();
  const kill = useKillInstance();

  return (
    <div className="mt-1 flex flex-col gap-1.5">
      <span className="text-muted-foreground px-1 text-[11px] font-medium">
        Running games ({running?.length ?? 0})
      </span>
      {(running ?? []).map((game) => (
        <div
          key={game.instanceId}
          className="flex items-center gap-2 rounded-lg border border-white/6 bg-black/15 px-3 py-1.5 text-[11px]"
        >
          <CheckCircle2 className="size-3.5 text-[var(--success)]" />
          <span className="min-w-0 flex-1 truncate">
            pid {game.pid ?? "?"}
            {game.connectAddress ? ` → ${game.connectAddress}` : ""}
          </span>
          <Button variant="ghost" size="sm" onClick={() => kill.mutate(game.instanceId)}>
            <Square className="size-3" /> Stop
          </Button>
        </div>
      ))}
      {(running ?? []).length === 0 ? (
        <p className="text-muted-foreground px-1 text-[11px]">No game is running.</p>
      ) : null}
    </div>
  );
}
