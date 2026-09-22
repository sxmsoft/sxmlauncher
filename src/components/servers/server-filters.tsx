import { Search, SlidersHorizontal } from "lucide-react";

import { Button } from "@/components/ui/button";
import { Input, Select } from "@/components/ui/input";
import { Switch } from "@/components/ui/switch";
import { cn } from "@/lib/utils";
import type { LoaderKind } from "@/types/instance";
import type { ServerFilter } from "@/types/server";

const LOADERS: LoaderKind[] = ["vanilla", "fabric", "quilt", "forge", "neoforge"];

/**
 * Filter bar for the global browser.
 *
 * Filters are applied locally *and* forwarded to the directory: the Redis query
 * narrows the set server-side (cheap), and the local predicate keeps results
 * honest when a listing changes between heartbeat and render.
 */
export function ServerFilters({
  filter,
  onChange,
  gameVersions,
  className,
}: {
  filter: ServerFilter;
  onChange: (filter: ServerFilter) => void;
  gameVersions: string[];
  className?: string;
}) {
  const patch = (next: Partial<ServerFilter>) => onChange({ ...filter, ...next });

  return (
    <div className={cn("flex flex-wrap items-center gap-2", className)}>
      <div className="relative min-w-64 flex-1">
        <Search className="text-muted-foreground pointer-events-none absolute top-1/2 left-3 size-4 -translate-y-1/2" />
        <Input
          value={filter.query ?? ""}
          onChange={(event) => patch({ query: event.target.value })}
          placeholder="Search worlds, hosts or tags…"
          className="pl-9"
        />
      </div>

      <Select
        value={filter.gameVersion ?? ""}
        onChange={(event) => patch({ gameVersion: event.target.value || undefined })}
        className="w-36"
      >
        <option value="">Any version</option>
        {gameVersions.map((version) => (
          <option key={version} value={version}>
            {version}
          </option>
        ))}
      </Select>

      <Select
        value={filter.loader ?? ""}
        onChange={(event) => patch({ loader: (event.target.value || undefined) as LoaderKind | undefined })}
        className="w-32"
      >
        <option value="">Any loader</option>
        {LOADERS.map((loader) => (
          <option key={loader} value={loader}>
            {loader}
          </option>
        ))}
      </Select>

      <Button
        variant={filter.hideFull ? "secondary" : "ghost"}
        size="sm"
        onClick={() => patch({ hideFull: !filter.hideFull })}
      >
        <SlidersHorizontal className="size-3.5" /> Hide full
      </Button>

      <label className="flex items-center gap-2 text-xs">
        <Switch
          checked={filter.hidePasswordProtected ?? false}
          onCheckedChange={(checked) => patch({ hidePasswordProtected: checked })}
        />
        Open worlds only
      </label>
    </div>
  );
}
