import { Download, Play, Zap } from "lucide-react";

import { Badge, StatusDot } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card } from "@/components/ui/card";
import { useInstallInstance, useLaunchInstance, useRunningInstances } from "@/hooks/queries";
import { cn, formatBytes, formatRelative, loaderTone } from "@/lib/utils";
import { useSessionsStore } from "@/stores/sessions";
import { isPlayable, STATUS_LABEL, type Instance } from "@/types/instance";

/** Compact tile: enough to pick an instance and start it, nothing more. */
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
    <Card
      className={cn(
        "group flex cursor-pointer flex-col gap-3 p-4 transition-all duration-200 hover:border-white/20",
        selected && "border-[color-mix(in_oklab,var(--primary)_50%,transparent)] ring-1 ring-[color-mix(in_oklab,var(--primary)_35%,transparent)]",
      )}
      onClick={onSelect}
    >
      <div className="flex items-start gap-3">
        <div
          className={cn(
            "flex size-10 shrink-0 items-center justify-center rounded-xl border border-white/10 bg-black/30 text-lg",
          )}
          aria-hidden
        >
          {instance.icon && instance.icon.length <= 4 ? instance.icon : "🎮"}
        </div>
        <div className="flex min-w-0 flex-1 flex-col gap-1">
          <div className="flex items-center gap-2">
            <span className="truncate text-sm font-semibold">{instance.name}</span>
            {isRunning ? <StatusDot tone="success" pulse /> : null}
            {hosting ? (
              <Badge variant="success">
                <Zap className="size-3" /> hosting
              </Badge>
            ) : null}
          </div>
          <span className="text-muted-foreground truncate text-xs">
            {instance.description || "No description"}
          </span>
        </div>
      </div>

      <div className="flex flex-wrap items-center gap-1.5">
        <Badge variant="outline">{instance.gameVersion}</Badge>
        <Badge className={cn("border", loaderTone(instance.loader.kind))}>{instance.loader.kind}</Badge>
        <Badge variant="outline">{instance.modCount} mods</Badge>
        {!playable ? <Badge variant="warning">{STATUS_LABEL[instance.status]}</Badge> : null}
      </div>

      <div className="text-muted-foreground flex items-center justify-between text-[11px]">
        <span>{formatBytes(instance.sizeBytes)}</span>
        <span>{instance.lastPlayedAt ? formatRelative(instance.lastPlayedAt) : "never played"}</span>
      </div>

      <div className="flex items-center gap-2" onClick={(event) => event.stopPropagation()}>
        {playable ? (
          <Button
            size="sm"
            className="flex-1"
            onClick={() => launch.mutate({ id: instance.id })}
            loading={launch.isPending}
            disabled={isRunning}
          >
            <Play className="size-3.5" />
            {isRunning ? "Running" : "Play"}
          </Button>
        ) : (
          <Button
            size="sm"
            variant="secondary"
            className="flex-1"
            onClick={() => install.mutate(instance.id)}
            loading={install.isPending}
          >
            <Download className="size-3.5" /> Install
          </Button>
        )}
        <Button size="sm" variant="ghost" onClick={onOpen}>
          Details
        </Button>
      </div>
    </Card>
  );
}
