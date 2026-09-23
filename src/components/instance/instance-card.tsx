import { Download, Play, Zap } from "lucide-react";

import { Badge, StatusDot } from "@/components/ui/badge";
import { useInstallInstance, useLaunchInstance, useRunningInstances } from "@/hooks/queries";
import { LOADER_FALLBACK_COLOR, loaderBg, loaderIcon } from "@/lib/loader-assets";
import { cn, formatRelative } from "@/lib/utils";
import { useSessionsStore } from "@/stores/sessions";
import { isPlayable, STATUS_LABEL, type Instance } from "@/types/instance";

/** Library tile: pick an instance, then play it. */
export function InstanceCard({
  instance,
  selected,
  onSelect,
  onOpen,
}: {
  instance: Instance;
  selected: boolean;
  onSelect: () => void;
  onOpen: () => void;
}) {
  const { data: running } = useRunningInstances();
  const hosting = useSessionsStore((state) => state.hostForInstance[instance.id]);
  const launch = useLaunchInstance();
  const install = useInstallInstance();

  const isRunning = (running ?? []).some((entry) => entry.instanceId === instance.id);
  const playable = isPlayable(instance);

  return (
    <article
      role="button"
      tabIndex={0}
      onClick={onSelect}
      onDoubleClick={onOpen}
      onKeyDown={(event) => {
        if (event.key === "Enter" || event.key === " ") {
          event.preventDefault();
          onSelect();
        }
      }}
      className={cn(
        "group glass flex min-h-[148px] cursor-pointer flex-col overflow-hidden rounded-[var(--radius)] text-left transition-[border-color,box-shadow,transform] duration-150",
        selected
          ? "border-[var(--rim-light)] shadow-[0_0_0_1px_var(--accent-dim),0_8px_28px_var(--accent-dim)]"
          : "hover:-translate-y-px hover:border-[var(--border-strong)]",
      )}
    >
      <div className="relative h-[84px] shrink-0 overflow-hidden" style={{ background: LOADER_FALLBACK_COLOR }}>
        <img
          src={loaderBg(instance.loader.kind)}
          alt=""
          className="absolute inset-0 size-full scale-110 object-cover blur-[3px] brightness-125"
        />
        <div className="absolute inset-0 bg-gradient-to-t from-black/45 via-black/10 to-transparent" />
        <img
          src={loaderIcon(instance.loader.kind, 64)}
          alt=""
          className="absolute bottom-2 left-3 size-16 drop-shadow-[0_6px_12px_rgba(0,0,0,0.45)]"
        />
      </div>
      <div className="flex flex-1 flex-col gap-3 p-4">
        <div className="min-w-0">
          <div className="flex items-center gap-2">
            <h3 className="truncate text-sm font-semibold">{instance.name}</h3>
            {isRunning ? <StatusDot tone="success" pulse /> : null}
            {hosting ? (
              <Badge variant="success">
                <Zap className="size-3" /> host
              </Badge>
            ) : null}
          </div>
          <p className="mt-1 truncate text-xs text-[var(--text-muted)]">
            {instance.description || "Isolated world"}
          </p>
          <div className="mt-2 flex flex-wrap gap-1.5">
            <span className="inline-flex h-[22px] items-center rounded-md border border-[var(--border)] bg-[var(--surface-3)] px-2 font-mono text-[11px] text-[var(--accent-soft)]">
              {instance.gameVersion}
            </span>
            <span className="inline-flex h-[22px] items-center rounded-md border border-[var(--border)] bg-[var(--surface-3)] px-2 font-mono text-[11px] text-[var(--text-muted)] capitalize">
              {instance.loader.kind}
            </span>
            {!playable ? <Badge variant="warning">{STATUS_LABEL[instance.status]}</Badge> : null}
          </div>
        </div>

        <div className="mt-auto flex items-center justify-between border-t border-[var(--border)] pt-3">
          <span className="text-[11px] text-[var(--text-faint)]">
            {instance.lastPlayedAt ? `Last played ${formatRelative(instance.lastPlayedAt)}` : "Never played"}
          </span>
          <button
            type="button"
            onClick={(event) => {
              event.stopPropagation();
              if (playable) launch.mutate({ id: instance.id });
              else install.mutate(instance.id);
            }}
            className="inline-flex h-[30px] items-center gap-1 rounded-lg border border-[rgba(52,211,153,0.25)] bg-[var(--success-dim)] px-3 text-xs font-semibold text-[var(--success)] opacity-0 transition-opacity group-hover:opacity-100 group-focus-within:opacity-100"
          >
            {playable ? <Play className="size-3 fill-current" /> : <Download className="size-3" />}
            {playable ? (isRunning ? "Running" : "Play") : "Install"}
          </button>
        </div>
      </div>
    </article>
  );
}
