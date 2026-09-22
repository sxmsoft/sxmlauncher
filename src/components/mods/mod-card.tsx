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
    <Card className={cn("flex flex-col overflow-hidden", className)}>
      <CardContent className="flex flex-1 flex-col gap-3 p-4">
        <div className="flex items-start gap-3">
          <div className="flex size-11 shrink-0 items-center justify-center overflow-hidden rounded-xl border border-white/10 bg-black/30">
            {hit.iconUrl ? (
              <img src={hit.iconUrl} alt="" className="size-full object-cover" loading="lazy" />
            ) : (
              <span className="text-sm font-semibold">{hit.title.slice(0, 2)}</span>
            )}
          </div>
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
