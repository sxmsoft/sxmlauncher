import type { ReactNode } from "react";

import { Download, History, Plus, Users } from "lucide-react";

import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardFooter } from "@/components/ui/card";
import { Hint } from "@/components/ui/tooltip";
import { cn, loaderTone } from "@/lib/utils";
import type { ModSearchHit } from "@/types/modpack";

/** Compact number: 84.2M, 1.4k. */
function compact(value: number): string {
  if (value >= 1_000_000) return `${(value / 1_000_000).toFixed(1)}M`;
  if (value >= 1_000) return `${(value / 1_000).toFixed(1)}k`;
  return String(value);
}

const COVERS = [
  "linear-gradient(135deg, #1a0a2e 0%, #4c1d95 40%, #7c3aed 70%, #1e1b4b 100%)",
  "linear-gradient(160deg, #0c1a12 0%, #14532d 35%, #22c55e 60%, #052e16 100%)",
  "linear-gradient(145deg, #1c0a0a 0%, #7f1d1d 30%, #ea580c 55%, #431407 100%)",
  "linear-gradient(135deg, #0a1628 0%, #1e3a8a 40%, #38bdf8 65%, #0c4a6e 100%)",
  "linear-gradient(150deg, #1a1020 0%, #831843 35%, #f472b6 55%, #4a044e 100%)",
  "linear-gradient(135deg, #111827 0%, #374151 40%, #9ca3af 70%, #1f2937 100%)",
];

function coverFor(id: string): string {
  let hash = 0;
  for (let index = 0; index < id.length; index += 1) hash = (hash + id.charCodeAt(index)) % COVERS.length;
  return COVERS[hash] ?? COVERS[0]!;
}

/** One search result, normalized across Modrinth and CurseForge. */
export function ModCard({
  hit,
  actionLabel,
  onInstall,
  onPickVersion,
  installing,
  extraActions,
  onOpenDetails,
  className,
}: {
  hit: ModSearchHit;
  actionLabel: string;
  onInstall: () => void;
  /** Opens the version picker — omitted when there is nothing to choose from. */
  onPickVersion?: () => void;
  installing: boolean;
  /** Icon actions rendered left of the install button (add-to-pack, download). */
  extraActions?: ReactNode;
  /** Opens the full project view (description + version history). */
  onOpenDetails?: () => void;
  className?: string;
}) {
  return (
    <Card className={cn("flex flex-col overflow-hidden p-0", className)}>
      <div className="relative h-[120px]" style={{ background: coverFor(hit.id) }}>
        {hit.iconUrl ? (
          <img src={hit.iconUrl} alt="" className="size-full object-cover" loading="lazy" />
        ) : null}
        <span className="absolute top-2.5 left-2.5 rounded-md border border-[var(--border)] bg-[color-mix(in_srgb,var(--bg-void)_70%,transparent)] px-2 py-0.5 text-[10px] font-semibold backdrop-blur-md">
          {hit.loaders[0] ?? (hit.source === "modrinth" ? "Modrinth" : "CurseForge")}
        </span>
      </div>
      <CardContent className="flex flex-1 flex-col gap-3 p-4">
        <div className="flex items-start gap-3">
          <div className="flex min-w-0 flex-1 flex-col gap-0.5">
            {onOpenDetails ? (
              <button
                type="button"
                className="truncate text-left text-sm font-semibold hover:underline"
                onClick={onOpenDetails}
              >
                {hit.title}
              </button>
            ) : (
              <span className="truncate text-sm font-semibold">{hit.title}</span>
            )}
            <span className="text-muted-foreground text-[11px]">
              {hit.source === "modrinth" ? "Modrinth" : "CurseForge"} · {hit.latestVersion ?? "—"}
            </span>
          </div>
          <Badge variant={hit.projectType === "modpack" ? "primary" : "outline"}>
            {hit.projectType}
          </Badge>
        </div>

        <p className="text-muted-foreground line-clamp-3 text-xs leading-relaxed">
          {hit.description}
        </p>

        <div className="flex flex-wrap gap-1.5">
          {hit.loaders.slice(0, 3).map((loader) => (
            <Badge key={loader} className={cn("border", loaderTone(loader))}>
              {loader}
            </Badge>
          ))}
          {hit.gameVersions.slice(0, 2).map((version) => (
            <Badge key={version} variant="outline">
              {version}
            </Badge>
          ))}
        </div>
      </CardContent>

      <CardFooter className="mt-auto justify-between border-t border-white/6 pt-3">
        <div className="text-muted-foreground flex items-center gap-3 text-[11px]">
          <span className="flex items-center gap-1">
            <Download className="size-3" /> {compact(hit.downloads)}
          </span>
          {hit.categories.length > 0 ? (
            <Hint label={hit.categories.join(", ")}>
              <span className="flex items-center gap-1">
                <Users className="size-3" /> {hit.categories[0]}
              </span>
            </Hint>
          ) : null}
        </div>
        <div className="flex items-center gap-1">
          {extraActions}
          {onPickVersion ? (
            <Hint label="Choose a specific version">
              <Button variant="ghost" size="icon-sm" onClick={onPickVersion}>
                <History className="size-3.5" />
                <span className="sr-only">Versions</span>
              </Button>
            </Hint>
          ) : null}
          <Button size="sm" onClick={onInstall} loading={installing}>
            <Plus className="size-3.5" /> {actionLabel}
          </Button>
        </div>
      </CardFooter>
    </Card>
  );
}
