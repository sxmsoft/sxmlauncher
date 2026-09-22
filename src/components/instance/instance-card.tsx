import { Download, Play, Zap } from "lucide-react";

import { Badge, StatusDot } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card } from "@/components/ui/card";
import { useInstallInstance, useLaunchInstance, useRunningInstances } from "@/hooks/queries";
import { cn, formatBytes, formatRelative } from "@/lib/utils";

const LOADER_STRIPE: Record<string, string> = {
  vanilla: "bg-[var(--primary)]",
  fabric: "bg-teal-400",
  quilt: "bg-sky-300",
  forge: "bg-orange-400",
  neoforge: "bg-lime-300",
};
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
        "group grid cursor-pointer grid-cols-[6px_minmax(0,1fr)_auto] items-stretch overflow-hidden",
        selected && "border-[var(--primary)]",
      )}
      onClick={onSelect}
    >
      <div
        className={cn("min-h-full", LOADER_STRIPE[instance.loader.kind] ?? "bg-[var(--primary)]")}
        aria-hidden
      />
      <div className="flex min-w-0 flex-col gap-1 px-4 py-3">
        <div className="flex items-center gap-2">
          <span className="font-display truncate text-lg leading-none">{instance.name}</span>
          {isRunning ? <StatusDot tone="success" pulse /> : null}
          {hosting ? (
            <Badge variant="success">
              <Zap className="size-3" /> host
            </Badge>
          ) : null}
        </div>
        <div className="text-muted-foreground flex flex-wrap items-center gap-x-3 gap-y-1 text-xs">
          <span className="uppercase tracking-wider">{instance.loader.kind}</span>
          <span>{instance.gameVersion}</span>
          <span>{instance.modCount} mods</span>
          <span>{formatBytes(instance.sizeBytes)}</span>
          <span>{instance.lastPlayedAt ? formatRelative(instance.lastPlayedAt) : "never played"}</span>
          {!playable ? <Badge variant="warning">{STATUS_LABEL[instance.status]}</Badge> : null}
        </div>
      </div>
      <div className="flex items-center gap-2 pr-3" onClick={(event) => event.stopPropagation()}>
        {playable ? (
          <Button
            size="sm"
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
            onClick={() => install.mutate(instance.id)}
            loading={install.isPending}
          >
            <Download className="size-3.5" /> Install
          </Button>
        )}
        <Button size="sm" variant="ghost" onClick={onOpen}>
          Open
        </Button>
      </div>
    </Card>
  );
}
