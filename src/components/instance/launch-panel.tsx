import { useMemo, useState } from "react";

import { ChevronRight, Download, FolderOpen, Play, Settings, Square, Trash } from "lucide-react";

import { HostPanel } from "@/components/instance/host-panel";
import { Button } from "@/components/ui/button";
import { Card } from "@/components/ui/card";
import { ConfirmDialog } from "@/components/ui/dialog";
import { IndeterminateProgress, Progress } from "@/components/ui/progress";
import { Hint } from "@/components/ui/tooltip";
import {
  useDeleteInstance,
  useInstallInstance,
  useInstanceMods,
  useKillInstance,
  useLaunchInstance,
  useRunningInstances,
} from "@/hooks/queries";
import { formatBytes, formatRelative, percent } from "@/lib/utils";
import { revealPath } from "@/lib/window";
import { useJobsStore } from "@/stores/jobs";
import { isPlayable, STATUS_LABEL, type Instance } from "@/types/instance";
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

  // Only the job for this instance. A create-then-install used to paint the
  // failure on whichever card was selected before.
  const job: ProgressEvent | null = useMemo(
    () =>
      jobs.find(
        (entry) =>
          entry.instanceId === instance.id &&
          (entry.kind === "instance_install" ||
            entry.kind === "modpack_install" ||
            entry.kind === "launch" ||
            entry.kind === "asset_hydration" ||
            entry.kind === "java_runtime"),
      ) ?? null,
    [jobs, instance.id],
  );

  const busy = job != null && !job.finished && !job.error;
  const installable = !isPlayable(instance);
  const mods = useInstanceMods(instance.id);
  const modRows = (mods.data ?? []).slice(0, 4);
  const ramGb = Math.max(1, Math.round(instance.memory.maxMb / 1024));

  return (
    <div className="grid items-start gap-5 xl:grid-cols-[minmax(0,1fr)_320px]">
      <Card className="relative min-h-[320px] overflow-hidden">
        <div
          aria-hidden
          className="pointer-events-none absolute inset-0"
          style={{
            background:
              "radial-gradient(ellipse 70% 60% at 70% 18%, var(--accent-glow), transparent 55%), linear-gradient(160deg, var(--surface-2), var(--surface-1))",
          }}
        />
        <div
          aria-hidden
          className="pointer-events-none absolute inset-0"
          style={{
            background: "linear-gradient(180deg, transparent 18%, color-mix(in srgb, var(--bg-void) 88%, transparent) 100%)",
          }}
        />
        <div className="relative flex h-full flex-col justify-end gap-4 p-7">
          <div className="inline-flex w-fit items-center gap-1.5 rounded-full border border-[rgba(52,211,153,0.25)] bg-[var(--success-dim)] px-2.5 py-1 text-[11px] font-medium text-[var(--success)]">
            <span className="size-1.5 rounded-full bg-[var(--success)] shadow-[0_0_8px_var(--success)]" />
            {isRunning ? "Playing" : installable ? STATUS_LABEL[instance.status] : "Ready"}
          </div>
          <div>
            <h2 className="text-[32px] leading-none font-bold tracking-[-0.03em]">{instance.name}</h2>
            <p className="mt-2 max-w-md text-[13px] text-[var(--text-muted)]">
              {instance.description || "Isolated instance — its own mods, configs, and worlds."}
            </p>
          </div>
          <div className="flex flex-wrap items-center gap-2">
            {isRunning ? (
              <Button
                variant="destructive"
                size="lg"
                className="h-[52px] min-w-44 rounded-xl px-7 text-[15px]"
                onClick={() => kill.mutate(instance.id)}
                loading={kill.isPending}
              >
                <Square /> Stop
              </Button>
            ) : installable ? (
              <Button
                size="lg"
                className="h-[52px] min-w-44 rounded-xl px-7 text-[15px]"
                onClick={() => install.mutate(instance.id)}
                loading={install.isPending || busy}
              >
                <Download /> Install
              </Button>
            ) : (
              <Button
                variant="success"
                size="lg"
                className="h-[52px] min-w-44 rounded-xl px-7 text-[15px]"
                onClick={() => launch.mutate({ id: instance.id })}
                loading={launch.isPending || busy}
              >
                <Play className="fill-current" /> Play
              </Button>
            )}
            <Button variant="outline" onClick={onOpenDetail}>
              Mods <ChevronRight className="size-3.5" />
            </Button>
            <Button variant="outline" onClick={onOpenSettings}>
              <Settings className="size-3.5" /> Settings
            </Button>
            <Hint label={instance.id}>
              <Button variant="outline" onClick={() => void revealPath(instance.id)}>
                <FolderOpen className="size-3.5" /> Open folder
              </Button>
            </Hint>
            <Button
              variant="ghost"
              className="hover:text-[var(--destructive)]"
              onClick={() => setConfirmRemove(true)}
            >
              <Trash className="size-3.5" /> Remove
            </Button>
          </div>
          <div className="mt-2 flex flex-wrap gap-x-6 gap-y-3">
            <Meta k="Version" v={instance.gameVersion} />
            <Meta
              k="Loader"
              v={`${instance.loader.kind}${instance.loader.version ? ` ${instance.loader.version}` : ""}`}
            />
            <Meta k="RAM" v={`${ramGb} GB`} />
            <Meta k="Java" v={String(instance.java.preferredMajor ?? instance.requiredJavaMajor)} />
          </div>

          {job ? (
          <div className="flex flex-col gap-2 rounded-xl border border-[var(--border)] bg-[color-mix(in_srgb,var(--bg-void)_55%,transparent)] p-3">
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
        </div>
      </Card>

      <div className="flex flex-col gap-3.5">
        <Card className="p-[18px]">
          <h3 className="mb-3.5 flex items-center justify-between text-[13px] font-semibold">
            Mods
            <span className="font-mono text-[11px] font-medium text-[var(--text-muted)]">{instance.modCount}</span>
          </h3>
          {modRows.length === 0 ? (
            <p className="text-xs text-[var(--text-muted)]">No mods installed yet.</p>
          ) : (
            modRows.map((mod) => (
              <div key={mod.projectId} className="flex items-center gap-2.5 border-b border-[var(--border)] py-2 last:border-b-0">
                <span
                  className="size-7 shrink-0 rounded-[7px] border border-[var(--border)]"
                  style={{ background: "linear-gradient(135deg, var(--surface-3), var(--accent-dim))" }}
                />
                <span className="min-w-0 flex-1 truncate text-xs font-medium">{mod.title || mod.fileName}</span>
                <span className="font-mono text-[10px] text-[var(--text-faint)]">
                  {mod.enabled ? "on" : "off"}
                </span>
              </div>
            ))
          )}
          <Button variant="outline" size="sm" className="mt-3 w-full" onClick={onOpenDetail}>
            View all
          </Button>
        </Card>

        <Card className="p-[18px]">
          <h3 className="mb-3.5 text-[13px] font-semibold">Resources</h3>
          <div className="grid grid-cols-2 gap-2.5">
            <StatTile k="Allocated" v={`${ramGb} GB`} />
            <StatTile
              k="Resolution"
              v={`${instance.resolution.width}×${instance.resolution.height}`}
            />
            <StatTile k="Last played" v={instance.lastPlayedAt ? formatRelative(instance.lastPlayedAt) : "Never"} />
            <StatTile k="Size" v={formatBytes(instance.sizeBytes)} />
          </div>
        </Card>
      </div>

      <Card className="p-4 xl:col-span-2">
        <p className="mb-3 text-xs leading-relaxed text-[var(--text-muted)]">
          LAN host for this instance stays here. The global P2P directory is paused on the Host screen.
        </p>
        <HostPanel instance={instance} />
      </Card>

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
    </div>
  );
}

function Meta({ k, v }: { k: string; v: string }) {
  return (
    <div className="flex flex-col gap-0.5">
      <span className="text-[10px] tracking-[0.08em] text-[var(--text-faint)] uppercase">{k}</span>
      <span className="font-mono text-[13px] capitalize">{v}</span>
    </div>
  );
}

function StatTile({ k, v }: { k: string; v: string }) {
  return (
    <div className="rounded-[var(--radius-md)] border border-[var(--border)] bg-[var(--surface-2)] p-3">
      <div className="text-[10px] tracking-[0.06em] text-[var(--text-faint)] uppercase">{k}</div>
      <div className="mt-1 font-mono text-sm font-semibold text-[var(--accent-soft)]">{v}</div>
    </div>
  );
}
