import { useMemo, useState } from "react";

import { ChevronRight, Download, FolderOpen, Play, Settings, Square, Trash } from "lucide-react";

import { HostPanel } from "@/components/instance/host-panel";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardFooter, CardHeader, CardTitle } from "@/components/ui/card";
import { ConfirmDialog } from "@/components/ui/dialog";
import { IndeterminateProgress, Progress } from "@/components/ui/progress";
import { Stat } from "@/components/ui/feedback";
import { Hint } from "@/components/ui/tooltip";
import {
  useDeleteInstance,
  useInstallInstance,
  useKillInstance,
  useLaunchInstance,
  useRunningInstances,
} from "@/hooks/queries";
import { cn, formatBytes, formatDuration, formatRelative, loaderTone, percent } from "@/lib/utils";
import { revealPath } from "@/lib/window";
import { useJobsStore } from "@/stores/jobs";
import { isPlayable, STATUS_LABEL, STATUS_TONE, type Instance } from "@/types/instance";
import { STAGE_LABEL, type ProgressEvent } from "@/types/modpack";

/**
 * The one card a player actually uses.
 *
 * Left: what this instance is (version, loader, memory, mods, playtime).
 * Right: the primary action — install, play, stop — with the live progress of
 * whatever the backend is currently doing for it, then the host toggle.
 */
export function LaunchPanel({
  instance,
  onOpenSettings,
  onOpenDetail,
}: {
  instance: Instance;
  onOpenSettings: () => void;
  onOpenDetail: () => void;
}) {
  const jobs = useJobsStore((state) => state.jobs);
  const { data: running } = useRunningInstances();

  const launch = useLaunchInstance();
  const install = useInstallInstance();
  const kill = useKillInstance();
  const remove = useDeleteInstance();

  const [confirmRemove, setConfirmRemove] = useState(false);

  const isRunning = useMemo(
    () => (running ?? []).some((entry) => entry.instanceId === instance.id),
    [running, instance.id],
  );

  // Newest job that could plausibly belong to this instance. The backend's event
  // carries no instance id (jobs are subsystem-scoped), so the card shows the
  // most recent launch/install job — which is exactly what the user just did.
  const job: ProgressEvent | null = useMemo(
    () =>
      jobs.find(
        (entry) =>
          entry.kind === "instance_install" ||
          entry.kind === "launch" ||
          entry.kind === "asset_hydration" ||
          entry.kind === "java_runtime",
      ) ?? null,
    [jobs],
  );

  const busy = job != null && !job.finished && !job.error;
  const installable = !isPlayable(instance);

  return (
    <Card className="overflow-hidden">
      <CardHeader className="flex-row items-start justify-between gap-4">
        <div className="flex min-w-0 flex-col gap-1">
          <CardTitle className="truncate text-lg">{instance.name}</CardTitle>
          <p className="text-muted-foreground truncate text-sm">
            {instance.description || "No description"}
          </p>
        </div>
        <div className="flex shrink-0 items-center gap-2">
          <Badge variant="outline">{instance.gameVersion}</Badge>
          <Badge className={cn("border", loaderTone(instance.loader.kind))}>
            {instance.loader.kind}
            {instance.loader.build ? ` ${instance.loader.build}` : ""}
          </Badge>
          <Badge variant={instance.status === "running" ? "success" : "outline"} className={STATUS_TONE[instance.status]}>
            {isRunning ? "playing" : STATUS_LABEL[instance.status]}
          </Badge>
        </div>
      </CardHeader>

      <CardContent className="flex flex-col gap-4">
        <div className="grid grid-cols-2 gap-4 sm:grid-cols-4">
          <Stat label="Mods" value={instance.modCount} />
          <Stat label="Memory" value={`${instance.memory.maxMb / 1024} GB`} />
          <Stat label="Playtime" value={formatDuration(instance.totalPlaytimeSecs)} />
          <Stat label="Size" value={formatBytes(instance.sizeBytes)} />
        </div>

        <div className="flex flex-wrap items-center gap-2 text-xs">
          <span className="text-muted-foreground">
            Java {instance.java.preferredMajor ?? instance.requiredJavaMajor}
          </span>
          <span className="text-muted-foreground">·</span>
          <span className="text-muted-foreground">
            {instance.resolution.width}×{instance.resolution.height}
            {instance.resolution.fullscreen ? " fullscreen" : ""}
          </span>
          <span className="text-muted-foreground">·</span>
          <span className="text-muted-foreground">
            {instance.lastPlayedAt ? `last played ${formatRelative(instance.lastPlayedAt)}` : "never played"}
          </span>
          {instance.sourcePack ? (
            <>
              <span className="text-muted-foreground">·</span>
              <span className="text-muted-foreground">from {instance.sourcePack.name}</span>
            </>
          ) : null}
        </div>

        {job ? (
          <div className="flex flex-col gap-2 rounded-xl border border-white/8 bg-black/20 p-3">
            <div className="flex items-center justify-between gap-3">
              <span className="text-sm font-medium">
                {STAGE_LABEL[job.stage]} · {job.label}
              </span>
              <span className="text-muted-foreground text-xs tabular-nums">
                {job.totalUnits > 0
                  ? `${Math.round(percent(job.completedUnits, job.totalUnits))}%`
                  : "working…"}
              </span>
            </div>
            {job.totalUnits > 0 ? (
              <Progress
                value={percent(job.completedUnits, job.totalUnits)}
                tone={job.error ? "destructive" : job.finished ? "success" : "primary"}
              />
            ) : (
              <IndeterminateProgress />
            )}
            <div className="text-muted-foreground flex items-center justify-between gap-3 text-[11px]">
              <span className="truncate">{job.currentItem ?? job.detail ?? "preparing"}</span>
              {job.bytesPerSecond > 0 ? (
                <span className="shrink-0 tabular-nums">{formatBytes(job.bytesPerSecond)}/s</span>
              ) : null}
            </div>
            {job.error ? (
              <p className="text-[var(--destructive)] text-xs leading-relaxed">{job.error}</p>
            ) : null}
          </div>
        ) : null}
      </CardContent>

      <CardFooter className="flex-wrap gap-2">
        {isRunning ? (
          <Button
            variant="destructive"
            size="xl"
            className="min-w-40"
            onClick={() => kill.mutate(instance.id)}
            loading={kill.isPending}
          >
            <Square /> Stop
          </Button>
        ) : installable ? (
          <Button
            size="xl"
            className="min-w-40"
            onClick={() => install.mutate(instance.id)}
            loading={install.isPending || busy}
          >
            <Download /> Install
          </Button>
        ) : (
          <Button
            size="xl"
            className="min-w-40"
            onClick={() => launch.mutate({ id: instance.id })}
            loading={launch.isPending || busy}
          >
            <Play /> Play
          </Button>
        )}

        <Button variant="ghost" size="sm" onClick={onOpenDetail}>
          Mods & world <ChevronRight className="size-3.5" />
        </Button>
        <Button variant="ghost" size="sm" onClick={onOpenSettings}>
          <Settings className="size-3.5" /> Settings
        </Button>
        <Hint label={instance.id}>
          <Button
            variant="ghost"
            size="sm"
            onClick={() => void revealPath(instance.id)}
          >
            <FolderOpen className="size-3.5" /> Folder
          </Button>
        </Hint>
        <Button
          variant="ghost"
          size="sm"
          className="ml-auto hover:text-[var(--destructive)]"
          onClick={() => setConfirmRemove(true)}
        >
          <Trash className="size-3.5" /> Remove
        </Button>
      </CardFooter>

      <CardContent className="pt-0">
        <HostPanel instance={instance} />
      </CardContent>

      <ConfirmDialog
        open={confirmRemove}
        onOpenChange={setConfirmRemove}
        title={`Remove “${instance.name}”?`}
        description="This deletes the instance and its mods, configs and worlds from disk. The Minecraft files can be reinstalled at any time."
        confirmLabel="Remove instance"
        destructive
        busy={remove.isPending}
        onConfirm={() =>
          remove.mutate(
            { id: instance.id, deleteFiles: true },
            { onSuccess: () => setConfirmRemove(false) },
          )
        }
      />
    </Card>
  );
}
