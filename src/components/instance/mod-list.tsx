import { useMemo, useState } from "react";

import { Package, Search, Trash } from "lucide-react";

import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { EmptyState, Skeleton } from "@/components/ui/feedback";
import { Input } from "@/components/ui/input";
import { Switch } from "@/components/ui/switch";
import { useInstanceMods, useToggleMod } from "@/hooks/queries";
import { formatRelative } from "@/lib/utils";

/**
 * Installed mods for one instance.
 *
 * Toggling writes `mods/<file>.disabled` on disk through the backend, which is
 * the same convention the loaders themselves understand — so a mod disabled here
 * stays disabled even if the player launches without the launcher.
 */
export function ModList({
  instanceId,
  onBrowseMods,
}: {
  instanceId: string;
  onBrowseMods: () => void;
}) {
  const { data: mods, isLoading } = useInstanceMods(instanceId);
  const toggle = useToggleMod();
  const [query, setQuery] = useState("");

  const filtered = useMemo(() => {
    const list = mods ?? [];
    if (!query.trim()) return list;
    const needle = query.trim().toLowerCase();
    return list.filter(
      (mod) =>
        mod.title.toLowerCase().includes(needle) ||
        mod.fileName.toLowerCase().includes(needle) ||
        mod.projectId.toLowerCase().includes(needle),
    );
  }, [mods, query]);

  return (
    <Card>
      <CardHeader className="flex-row items-center justify-between gap-4">
        <CardTitle className="flex items-center gap-2">
          <Package className="size-4" /> Installed mods
          {mods ? <Badge variant="outline">{mods.length}</Badge> : null}
        </CardTitle>
        <div className="flex items-center gap-2">
          <div className="relative">
            <Search className="text-muted-foreground pointer-events-none absolute top-1/2 left-3 size-3.5 -translate-y-1/2" />
            <Input
              value={query}
              onChange={(event) => setQuery(event.target.value)}
              placeholder="Filter mods"
              className="h-9 w-44 pl-8 text-xs"
            />
          </div>
          <Button size="sm" variant="outline" onClick={onBrowseMods}>
            Add mods
          </Button>
        </div>
      </CardHeader>

      <CardContent className="flex flex-col gap-2">
        {isLoading
          ? [0, 1, 2].map((index) => <Skeleton key={index} className="h-12 w-full" />)
          : null}

        {!isLoading && filtered.length === 0 ? (
          <EmptyState
            icon={<Package />}
            title={mods && mods.length > 0 ? "No mods match that filter" : "No mods installed"}
            description={
              mods && mods.length > 0
                ? undefined
                : "Search Modrinth or CurseForge and install into this instance — dependencies come along automatically."
            }
            action={
              <Button size="sm" onClick={onBrowseMods}>
                Browse mods
              </Button>
            }
          />
        ) : null}

        {filtered.map((mod) => (
          <div
            key={`${mod.source}-${mod.projectId}`}
            className="flex items-center gap-3 rounded-lg border border-white/8 bg-black/20 px-3 py-2"
          >
            <div className="flex min-w-0 flex-1 flex-col gap-0.5">
              <span className="flex items-center gap-2 truncate text-sm font-medium">
                {mod.title}
                <Badge variant={mod.source === "modrinth" ? "primary" : "warning"}>
                  {mod.source === "modrinth" ? "Modrinth" : "CurseForge"}
                </Badge>
              </span>
              <span className="text-muted-foreground truncate text-[11px]">
                {mod.fileName} · added {formatRelative(mod.installedAt)}
              </span>
            </div>
            <Switch
              checked={mod.enabled}
              onCheckedChange={(enabled) =>
                toggle.mutate({ id: instanceId, projectId: mod.projectId, enabled })
              }
              aria-label={`${mod.enabled ? "Disable" : "Enable"} ${mod.title}`}
            />
            <Button
              variant="ghost"
              size="icon-sm"
              className="hover:text-[var(--destructive)]"
              onClick={() => toggle.mutate({ id: instanceId, projectId: mod.projectId, enabled: false })}
            >
              <Trash className="size-3.5" />
              <span className="sr-only">Disable</span>
            </Button>
          </div>
        ))}
      </CardContent>
    </Card>
  );
}
