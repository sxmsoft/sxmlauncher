import { useMemo, useState } from "react";

import { Download, FolderOpen, Play, Settings, Square, Trash } from "lucide-react";
import { useTranslation } from "react-i18next";

import { HeroBackdrop } from "@/components/home/hero-backdrop";
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
import { translateActivityStatus } from "@/i18n/status";
import { activityPresentation } from "@/lib/activity";
import { formatBytes, formatRelative } from "@/lib/utils";
import { revealPath } from "@/lib/window";
import { useJobsStore } from "@/stores/jobs";
import { isPlayable, STATUS_LABEL, type Instance } from "@/types/instance";
import type { ProgressEvent } from "@/types/modpack";

/**
 * Home hero: blurred world, a large Play control, and version / configure pills.
 * Progress appears only while a job is actually working.
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
  const { t } = useTranslation();
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

  const view = job ? activityPresentation(job) : null;
  const busy = view?.mode === "work";
  const installable = !isPlayable(instance);
  const mods = useInstanceMods(instance.id);
  const modRows = (mods.data ?? []).slice(0, 4);
  const ramGb = Math.max(1, Math.round(instance.memory.maxMb / 1024));
  const loader = `${instance.loader.kind}${instance.loader.version ? ` ${instance.loader.version}` : ""}`;

  return (
    <div className="flex flex-col gap-4">
      <div className="grid items-stretch gap-4 xl:grid-cols-[minmax(0,1fr)_300px]">
        <section className="relative min-h-[460px] overflow-hidden rounded-[28px] border border-white/10 shadow-[0_24px_80px_rgba(0,0,0,0.45)]">
          <HeroBackdrop />
          <div className="relative flex min-h-[460px] flex-col justify-end gap-5 p-8">
            <span className="glass-pill inline-flex w-fit items-center gap-2 px-3 py-1 text-[11px] font-medium text-white/90">
              <span
                className="size-1.5 rounded-full"
                style={{
                  background: isRunning ? "var(--success)" : "var(--accent)",
                  boxShadow: "0 0 8px var(--accent)",
                }}
              />
              {isRunning ? t("home.playing") : installable ? STATUS_LABEL[instance.status] : t("home.ready")}
            </span>
            <div>
              <h2 className="text-[40px] leading-none font-bold tracking-[-0.04em] text-white">
                {instance.name}
              </h2>
              <p className="mt-3 max-w-lg text-sm text-white/70">
                {instance.description || t("home.emptyBody")}
              </p>
            </div>
            <div className="flex flex-wrap items-center gap-3">
              {isRunning ? (
                <Button
                  variant="destructive"
                  className="h-14 min-w-[168px] rounded-2xl px-8 text-base tracking-[0.16em] uppercase"
                  onClick={() => kill.mutate(instance.id)}
                  loading={kill.isPending}
                >
                  <Square /> {t("home.stop")}
                </Button>
              ) : installable ? (
                <Button
                  className="h-14 min-w-[168px] rounded-2xl px-8 text-base tracking-[0.16em] uppercase"
                  onClick={() => install.mutate(instance.id)}
                  loading={install.isPending || busy}
                >
                  <Download /> {t("home.install")}
                </Button>
              ) : (
                <Button
                  className="h-14 min-w-[180px] rounded-2xl px-10 text-base tracking-[0.22em] uppercase"
                  onClick={() => launch.mutate({ id: instance.id })}
                  loading={launch.isPending || busy}
                >
                  <Play className="size-4" strokeWidth={1.75} /> {t("home.play")}
                </Button>
              )}
              <button
                type="button"
                onClick={onOpenDetail}
                className="glass-pill inline-flex h-12 items-center gap-2 px-4 text-sm font-medium text-white"
              >
                <span className="text-[11px] tracking-wide text-white/60 uppercase">{t("home.version")}</span>
                <span className="font-mono">{instance.gameVersion}</span>
              </button>
              <button
                type="button"
                onClick={onOpenSettings}
                className="glass-pill inline-flex h-12 items-center gap-2 px-4 text-sm font-medium text-white"
              >
                <Settings className="size-4" strokeWidth={1.5} />
                {t("home.configure")}
              </button>
            </div>

            {view && job ? (
              <div className="glass-pill flex max-w-xl flex-col gap-2 rounded-2xl px-4 py-3 text-white">
                <div className="flex items-center justify-between gap-3 text-sm">
                  <span className={view.mode === "error" ? "text-[var(--danger)]" : ""}>
                    {translateActivityStatus(view.status, t)} · {job.label}
                  </span>
                  {view.showProgress && view.progress != null ? (
                    <span className="font-mono text-xs text-white/70">{Math.round(view.progress)}%</span>
                  ) : null}
                </div>
                {view.showProgress ? (
                  view.progress == null ? (
                    <IndeterminateProgress />
                  ) : (
                    <Progress value={view.progress} tone={view.mode === "work" ? "primary" : "primary"} />
                  )
                ) : null}
                {view.mode === "error" && job.error ? (
                  <p className="text-xs text-[var(--danger)]">{job.error}</p>
                ) : null}
              </div>
            ) : null}
          </div>
        </section>

        <div className="flex flex-col gap-3.5">
          <Card className="p-[18px]">
            <h3 className="mb-3.5 flex items-center justify-between text-[13px] font-semibold">
              {t("home.mods")}
              <span className="font-mono text-[11px] font-medium text-[var(--text-muted)]">{instance.modCount}</span>
            </h3>
            {modRows.length === 0 ? (
              <p className="text-xs text-[var(--text-muted)]">{t("home.noMods")}</p>
            ) : (
              modRows.map((mod) => (
                <div
                  key={mod.projectId}
                  className="flex items-center gap-2.5 border-b border-white/8 py-2 last:border-b-0"
                >
                  <span
                    className="size-7 shrink-0 rounded-[7px] border border-white/10"
                    style={{ background: "linear-gradient(135deg, var(--surface-3), var(--accent-dim))" }}
                  />
                  <span className="min-w-0 flex-1 truncate text-xs font-medium">{mod.title || mod.fileName}</span>
                  <span className="font-mono text-[10px] text-[var(--text-faint)]">{mod.enabled ? "on" : "off"}</span>
                </div>
              ))
            )}
            <Button variant="outline" size="sm" className="mt-3 w-full rounded-full" onClick={onOpenDetail}>
              {t("home.viewAll")}
            </Button>
          </Card>

          <Card className="p-[18px]">
            <h3 className="mb-3.5 text-[13px] font-semibold">{t("home.resources")}</h3>
            <div className="grid grid-cols-2 gap-2.5">
              <StatTile k={t("home.allocated")} v={`${ramGb} GB`} />
              <StatTile k={t("home.resolution")} v={`${instance.resolution.width}×${instance.resolution.height}`} />
              <StatTile
                k={t("home.lastPlayed")}
                v={instance.lastPlayedAt ? formatRelative(instance.lastPlayedAt) : t("home.never")}
              />
              <StatTile k={t("home.size")} v={formatBytes(instance.sizeBytes)} />
            </div>
            <div className="mt-3 flex flex-wrap gap-2">
              <Hint label={instance.id}>
                <Button variant="ghost" size="sm" onClick={() => void revealPath(instance.id)}>
                  <FolderOpen className="size-3.5" /> {t("home.openFolder")}
                </Button>
              </Hint>
              <Button
                variant="ghost"
                size="sm"
                className="hover:text-[var(--destructive)]"
                onClick={() => setConfirmRemove(true)}
              >
                <Trash className="size-3.5" /> {t("home.remove")}
              </Button>
            </div>
            <p className="mt-3 text-[11px] text-[var(--text-faint)]">
              {t("home.loader")} · {loader} · {t("home.java")}{" "}
              {instance.java.preferredMajor ?? instance.requiredJavaMajor}
            </p>
          </Card>
        </div>
      </div>

      <Card className="p-4">
        <p className="mb-3 text-xs leading-relaxed text-[var(--text-muted)]">{t("home.hostNote")}</p>
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

function StatTile({ k, v }: { k: string; v: string }) {
  return (
    <div className="rounded-2xl border border-white/8 bg-black/20 p-3">
      <div className="text-[10px] tracking-[0.06em] text-[var(--text-faint)] uppercase">{k}</div>
      <div className="mt-1 font-mono text-sm font-semibold text-[var(--accent-soft)]">{v}</div>
    </div>
  );
}
