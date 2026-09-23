import { ArrowDownToLine, Ban, CheckCircle2, ListVideo, Square, X } from "lucide-react";
import { useTranslation } from "react-i18next";

import { Badge, StatusDot } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { IndeterminateProgress, Progress } from "@/components/ui/progress";
import { useKillInstance, useRunningInstances } from "@/hooks/queries";
import { translateActivityStatus } from "@/i18n/status";
import { activityPresentation } from "@/lib/activity";
import { aggregateProgress, useJobsStore } from "@/stores/jobs";
import { useUiStore } from "@/stores/ui";
import type { JobKind, ProgressEvent } from "@/types/modpack";

const KIND_KEY: Record<JobKind, string> = {
  instance_install: "activity.kind.instance_install",
  modpack_install: "activity.kind.modpack_install",
  mod_download: "activity.kind.mod_download",
  java_runtime: "activity.kind.java_runtime",
  asset_hydration: "activity.kind.asset_hydration",
  launch: "activity.kind.launch",
  p2p_connect: "activity.kind.p2p_connect",
  p2p_host: "activity.kind.p2p_host",
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
  const { t } = useTranslation();

  if (!open) return null;

  return (
    <div className="pointer-events-none absolute inset-0 z-40">
      <button
        aria-label={t("activity.close")}
        className="pointer-events-auto absolute inset-0 bg-black/50 backdrop-blur-[2px]"
        onClick={() => setOpen(false)}
      />
      <aside className="glass-strong pointer-events-auto absolute top-0 right-0 flex h-full w-[26rem] max-w-[92vw] flex-col gap-3 rounded-l-[28px] border-white/10 p-4 shadow-2xl">
        <ActivityFeed onClose={() => setOpen(false)} />
      </aside>
    </div>
  );
}

export function ActivityFeed({
  embedded = false,
  onClose,
}: {
  embedded?: boolean;
  onClose?: () => void;
}) {
  const { t } = useTranslation();
  const jobs = useJobsStore((state) => state.jobs);
  const dismiss = useJobsStore((state) => state.dismiss);
  const clearFinished = useJobsStore((state) => state.clearFinished);
  const { data: runningGames } = useRunningInstances();

  const active = jobs.filter((job) => !job.finished && !job.error);
  const done = jobs.filter((job) => job.finished || job.error);
  const views = active.map((job) => activityPresentation(job));
  const workCount = views.filter((view) => view.mode === "work").length;
  const steadyCount = views.filter((view) => view.mode === "steady").length;
  const errorCount = views.filter((view) => view.mode === "error").length;
  const aggregate = aggregateProgress(active.filter((_, index) => views[index]?.mode === "work"));
  const gameCount = runningGames?.length ?? 0;

  return (
    <div className={embedded ? "glass flex flex-col gap-3 rounded-[28px] p-4" : "flex min-h-0 flex-1 flex-col gap-3"}>
      <div className="flex items-center gap-2">
        <ListVideo className="size-4 text-[var(--accent)]" strokeWidth={1.5} />
        <h2 className="flex-1 text-sm font-semibold tracking-tight">{t("activity.title")}</h2>
        {workCount > 0 ? (
          <Badge variant="primary">
            {t("activity.active", { count: workCount })}
            {aggregate.total > 0 ? ` · ${Math.round(aggregate.percent)}%` : ""}
          </Badge>
        ) : steadyCount > 0 ? (
          <Badge variant="success">{t("activity.running", { count: steadyCount })}</Badge>
        ) : gameCount > 0 ? (
          <Badge variant="success">{t("activity.running", { count: gameCount })}</Badge>
        ) : errorCount > 0 ? (
          <Badge variant="destructive">{t("activity.failedCount", { count: errorCount })}</Badge>
        ) : (
          <Badge variant="outline">{t("activity.idle")}</Badge>
        )}
        {onClose ? (
          <Button variant="ghost" size="icon-sm" onClick={onClose}>
            <X className="size-4" />
          </Button>
        ) : null}
      </div>

      <div className="flex min-h-0 flex-1 flex-col gap-2 overflow-y-auto pr-1">
        {active.length === 0 && gameCount === 0 ? (
          <div className="flex flex-col items-center gap-2 rounded-2xl border border-white/8 bg-black/20 px-4 py-8 text-center">
            <ArrowDownToLine className="size-6 text-[var(--muted-foreground)]" strokeWidth={1.5} />
            <p className="text-sm font-medium">{t("activity.emptyTitle")}</p>
            <p className="text-muted-foreground text-xs leading-relaxed">{t("activity.emptyBody")}</p>
          </div>
        ) : (
          active.map((job) => <ActiveJobRow key={job.jobId} job={job} />)
        )}

        {done.length > 0 ? (
          <div className="mt-1 flex flex-col gap-1.5">
            <div className="flex items-center gap-2 px-1">
              <span className="text-muted-foreground text-[11px] font-medium">
                {t("activity.recent", { count: done.length })}
              </span>
              <div className="flex-1" />
              <Button variant="ghost" size="sm" onClick={clearFinished}>
                {t("activity.clear")}
              </Button>
            </div>
            {done.slice(0, 8).map((job) => (
              <div
                key={job.jobId}
                className="flex items-center gap-2 rounded-xl border border-white/6 bg-black/15 px-3 py-1.5"
              >
                <StatusDot tone={job.error ? "destructive" : "success"} />
                <span className="text-muted-foreground min-w-0 flex-1 truncate text-[11px]">
                  {job.error && !job.error.includes("cancelled")
                    ? `${job.label} — ${job.error}`
                    : `${job.label} · ${job.error ? t("activity.stopped") : t("activity.done")}`}
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
    </div>
  );
}

function ActiveJobRow({ job }: { job: ProgressEvent }) {
  const { t } = useTranslation();
  const cancelling = useJobsStore((state) => state.cancelling);
  const cancel = useJobsStore((state) => state.cancel);
  const stopping = cancelling.includes(job.jobId);
  const view = activityPresentation(job);
  const extra =
    view.mode === "work"
      ? (job.currentItem ?? job.detail)
      : view.mode === "steady"
        ? job.detail
        : null;
  const status = stopping ? t("activity.stopping") : translateActivityStatus(view.status, t);
  const showExtra = extra != null && extra !== "" && extra !== status && extra !== job.label;

  return (
    <div
      className={
        "flex flex-col gap-1.5 rounded-xl border bg-black/25 p-3 " +
        (view.mode === "error"
          ? "border-[color-mix(in_srgb,var(--destructive)_45%,transparent)]"
          : "border-white/8")
      }
    >
      <div className="flex items-center gap-2">
        <StatusDot
          tone={view.mode === "error" ? "destructive" : view.mode === "steady" ? "success" : "primary"}
          pulse={view.mode === "steady"}
        />
        <span
          className={
            "min-w-0 flex-1 truncate text-xs font-medium " +
            (view.mode === "error" ? "text-[var(--destructive)]" : "")
          }
        >
          {job.label}
        </span>
        <Badge variant={view.mode === "error" ? "destructive" : view.mode === "steady" ? "success" : "outline"}>
          {view.mode === "error" ? t("activity.failed") : t(KIND_KEY[job.kind])}
        </Badge>
        <Button
          variant="ghost"
          size="icon-sm"
          disabled={stopping}
          onClick={() => void cancel(job.jobId)}
          title={t("activity.stopJob")}
        >
          <Ban className="size-3.5" />
        </Button>
      </div>
      <div
        className={
          "truncate text-[11px] " +
          (view.mode === "error"
            ? "text-[var(--destructive)]"
            : view.mode === "steady"
              ? "text-[var(--success)]"
              : "text-muted-foreground")
        }
      >
        {status}
        {showExtra ? ` · ${extra}` : ""}
      </div>
      {view.showProgress ? (
        view.progress == null ? (
          <IndeterminateProgress className="h-1.5" />
        ) : (
          <div className="flex items-center gap-2">
            <Progress value={view.progress} className="h-1.5 flex-1" />
            <span className="text-muted-foreground shrink-0 text-[10px] tabular-nums">
              {Math.round(view.progress)}%
            </span>
          </div>
        )
      ) : null}
      {view.mode === "work" && job.bytesPerSecond > 0 ? (
        <div className="text-muted-foreground text-[10px] tabular-nums">
          {(job.bytesPerSecond / 1_048_576).toFixed(1)} MiB/s
        </div>
      ) : null}
    </div>
  );
}

function RunningGames() {
  const { t } = useTranslation();
  const { data: running } = useRunningInstances();
  const kill = useKillInstance();

  return (
    <div className="mt-1 flex flex-col gap-1.5">
      <span className="text-muted-foreground px-1 text-[11px] font-medium">
        {t("activity.games", { count: running?.length ?? 0 })}
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
            <Square className="size-3" /> {t("activity.stop")}
          </Button>
        </div>
      ))}
      {(running ?? []).length === 0 ? (
        <p className="text-muted-foreground px-1 text-[11px]">{t("activity.noGame")}</p>
      ) : null}
    </div>
  );
}
