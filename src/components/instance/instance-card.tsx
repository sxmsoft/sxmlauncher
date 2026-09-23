import { Download, Play, Zap } from "lucide-react";
import { useTranslation } from "react-i18next";

import { Badge, StatusDot } from "@/components/ui/badge";
import { useInstallInstance, useLaunchInstance, useRunningInstances } from "@/hooks/queries";
import { cn, formatRelative } from "@/lib/utils";
import { useSessionsStore } from "@/stores/sessions";
import { isPlayable, STATUS_LABEL, type Instance, type LoaderKind } from "@/types/instance";

const COVER: Record<LoaderKind, string> = {
  vanilla: "linear-gradient(180deg, #5a9c3a 0 38%, #8b6b3a 38% 48%, #6b5230 48%)",
  fabric: "linear-gradient(145deg, #4c1d95 0%, #8b5cf6 45%, #2e1065 100%)",
  quilt: "linear-gradient(180deg, #7dd3fc 0 35%, #86efac 35% 55%, #fef08a 55%)",
  forge: "linear-gradient(145deg, #1e3a5f 0%, #3b82f6 50%, #0f172a 100%)",
  neoforge: "linear-gradient(135deg, #312e81 0%, #6366f1 40%, #a855f7 100%)",
};

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
  const { t } = useTranslation();
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
        "group glass flex min-h-[148px] cursor-pointer flex-col gap-3 rounded-[var(--radius)] p-4 text-left transition-[border-color,box-shadow,transform] duration-150",
        selected
          ? "border-[var(--rim-light)] shadow-[0_0_0_1px_var(--accent-dim),0_8px_28px_var(--accent-dim)]"
          : "hover:-translate-y-px hover:border-[var(--border-strong)]",
      )}
    >
      <div className="flex items-start gap-3">
        <div
          className="size-12 shrink-0 rounded-xl border border-[var(--border)] shadow-[inset_0_1px_0_rgba(255,255,255,0.08)]"
          style={{ background: COVER[instance.loader.kind] ?? COVER.vanilla }}
          aria-hidden
        />
        <div className="min-w-0 flex-1">
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
      </div>

      <div className="mt-auto flex items-center justify-between border-t border-[var(--border)] pt-3">
        <span className="text-[11px] text-[var(--text-faint)]">
          {instance.lastPlayedAt
            ? `${t("home.lastPlayed")} ${formatRelative(instance.lastPlayedAt)}`
            : t("library.neverPlayed")}
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
    </article>
  );
}
