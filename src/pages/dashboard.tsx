import { useMemo, useState } from "react";

import { useNavigate } from "react-router-dom";

import { CirclePlay, FolderSearch, Link2, Plus } from "lucide-react";

import { PageHeader } from "@/components/common/page-header";
import { CreateInstanceDialog } from "@/components/instance/create-instance-dialog";
import { InstanceCard } from "@/components/instance/instance-card";
import { InstanceSettingsDialog } from "@/components/instance/instance-settings-dialog";
import { LaunchPanel } from "@/components/instance/launch-panel";
import { JoinCodeDialog } from "@/components/servers/join-code-dialog";
import { MySessions } from "@/components/servers/my-sessions";
import { Button } from "@/components/ui/button";
import { Card } from "@/components/ui/card";
import { CardSkeleton, EmptyState, ErrorState } from "@/components/ui/feedback";
import { useImportInstance, useInstances, useSelectedInstance } from "@/hooks/queries";
import { cn } from "@/lib/utils";
import { useUiStore } from "@/stores/ui";
import type { LoaderKind } from "@/types/instance";

const FILTERS: Array<{ id: "all" | LoaderKind | "modded"; label: string }> = [
  { id: "all", label: "All" },
  { id: "vanilla", label: "Vanilla" },
  { id: "fabric", label: "Fabric" },
  { id: "forge", label: "Forge" },
  { id: "modded", label: "Modded" },
];

/**
 * Play. The hero launches the selected instance; the grid underneath is the
 * library. Both live on `/` so the existing instance routes stay intact.
 */
export function DashboardPage() {
  const navigate = useNavigate();
  const instances = useInstances();
  const selected = useSelectedInstance();
  const select = useUiStore((state) => state.selectInstance);
  const query = useUiStore((state) => state.chromeQuery);
  const importFolder = useImportInstance();

  const [createOpen, setCreateOpen] = useState(false);
  const [settingsOpen, setSettingsOpen] = useState(false);
  const [joinOpen, setJoinOpen] = useState(false);
  const [filter, setFilter] = useState<(typeof FILTERS)[number]["id"]>("all");

  const visible = useMemo(() => {
    const needle = query.trim().toLowerCase();
    return (instances.data ?? []).filter((instance) => {
      if (needle && !`${instance.name} ${instance.gameVersion} ${instance.loader.kind}`.toLowerCase().includes(needle)) {
        return false;
      }
      if (filter === "all") return true;
      if (filter === "modded") return instance.loader.kind !== "vanilla";
      return instance.loader.kind === filter;
    });
  }, [instances.data, filter, query]);

  return (
    <div className="flex flex-col gap-6">
      <PageHeader
        title="Play"
        description="Active instance — start when it's ready."
        actions={
          <>
            <Button variant="outline" size="sm" onClick={() => setJoinOpen(true)}>
              <Link2 className="size-4" /> Join with code
            </Button>
            <Button variant="outline" size="sm" onClick={() => importFolder.mutate()} loading={importFolder.isPending}>
              <FolderSearch className="size-4" /> Import
            </Button>
            <Button size="sm" onClick={() => setCreateOpen(true)}>
              <Plus className="size-4" /> New instance
            </Button>
          </>
        }
      />

      {instances.isError ? (
        <ErrorState
          title="Could not load instances"
          message={instances.error instanceof Error ? instances.error.message : String(instances.error)}
          onRetry={() => void instances.refetch()}
        />
      ) : null}

      {instances.isLoading ? (
        <CardSkeleton className="h-80" />
      ) : selected ? (
        <LaunchPanel
          instance={selected}
          onOpenSettings={() => setSettingsOpen(true)}
          onOpenDetail={() => navigate(`/instances/${selected.id}`)}
        />
      ) : (
        <Card>
          <EmptyState
            icon={<CirclePlay />}
            title="Create your first instance"
            description="An instance is an isolated game environment: its own mods, configs, resource packs and worlds."
            action={
              <Button onClick={() => setCreateOpen(true)}>
                <Plus /> New instance
              </Button>
            }
          />
        </Card>
      )}

      <section className="flex flex-col gap-3.5">
        <div className="flex flex-wrap items-end justify-between gap-3">
          <div>
            <h2 className="text-[22px] font-semibold tracking-[-0.02em]">Instances</h2>
            <p className="mt-1 text-[13px] text-[var(--text-muted)]">Installed instances — select, play, manage.</p>
          </div>
        </div>
        <div className="flex flex-wrap gap-1.5">
          {FILTERS.map((entry) => (
            <button
              key={entry.id}
              type="button"
              onClick={() => setFilter(entry.id)}
              className={cn(
                "h-[30px] rounded-lg px-3 text-xs font-medium text-[var(--text-muted)]",
                filter === entry.id
                  ? "border border-[var(--border)] bg-[var(--accent-dim)] text-[var(--text)]"
                  : "hover:bg-[var(--accent-dim)] hover:text-[var(--text)]",
              )}
            >
              {entry.label}
            </button>
          ))}
        </div>
        {visible.length === 0 ? (
          <p className="text-sm text-[var(--text-muted)]">No instances match this filter.</p>
        ) : (
          <div className="grid items-start gap-3.5 sm:grid-cols-2 xl:grid-cols-3">
            {visible.map((instance) => (
              <InstanceCard
                key={instance.id}
                instance={instance}
                selected={instance.id === selected?.id}
                onSelect={() => select(instance.id)}
                onOpen={() => navigate(`/instances/${instance.id}`)}
              />
            ))}
          </div>
        )}
        {selected ? (
          <div className="glass flex flex-wrap items-center justify-between gap-3 rounded-[var(--radius)] px-4 py-3.5">
            <div className="flex items-center gap-3 text-[13px] text-[var(--text-muted)]">
              <span className="inline-flex h-[22px] items-center rounded-md border border-[var(--border)] bg-[var(--surface-3)] px-2 font-mono text-[11px] text-[var(--accent-soft)]">
                Selected
              </span>
              <span>
                <strong className="font-semibold text-[var(--text)]">{selected.name}</strong>
                {" · "}
                {selected.gameVersion} {selected.loader.kind} · {selected.modCount} mods
              </span>
            </div>
            <Button
              variant="success"
              onClick={() => navigate(`/instances/${selected.id}`)}
            >
              Open
            </Button>
          </div>
        ) : null}
      </section>

      <MySessions />

      <CreateInstanceDialog open={createOpen} onOpenChange={setCreateOpen} />
      {selected ? (
        <InstanceSettingsDialog instance={selected} open={settingsOpen} onOpenChange={setSettingsOpen} />
      ) : null}
      <JoinCodeDialog open={joinOpen} onOpenChange={setJoinOpen} />
    </div>
  );
}
