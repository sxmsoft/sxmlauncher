import { History } from "lucide-react";

import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Skeleton } from "@/components/ui/feedback";
import { useModAllVersions, useModProject } from "@/hooks/queries";
import { cn, formatBytes, formatRelative, loaderTone } from "@/lib/utils";
import type { ModSearchHit, ModVersion } from "@/types/modpack";

/**
 * Modpack detail: description, categories and the full version history with
 * a one-click install per row. Opened from the search card, so browsing a
 * pack before committing disk space is the default path, not an extra.
 */
export function ModpackDetailDialog({
  hit,
  onClose,
  installing,
  onInstall,
}: {
  hit: ModSearchHit | null;
  onClose: () => void;
  installing: boolean;
  onInstall: (versionId?: string) => void;
}) {
  const open = hit != null;
  const { data: project } = useModProject(hit?.id ?? null, hit?.source ?? "modrinth");
  const { data: versions, isLoading: versionsLoading } = useModAllVersions(
    hit?.id ?? null,
    hit?.source ?? "modrinth",
    open,
  );

  return (
    <Dialog open={open} onOpenChange={(next) => !next && onClose()}>
      <DialogContent className="max-h-[85vh] max-w-2xl overflow-y-auto">
        {hit == null ? null : (
          <>
            <DialogHeader>
              <div className="flex items-start gap-3">
                <div className="flex size-14 shrink-0 items-center justify-center overflow-hidden rounded-xl border border-white/10 bg-black/30">
                  {hit.iconUrl ? (
                    <img src={hit.iconUrl} alt="" className="size-full object-cover" />
                  ) : (
                    <span className="text-lg font-semibold">{hit.title.slice(0, 2)}</span>
                  )}
                </div>
                <div className="flex min-w-0 flex-col gap-1">
                  <DialogTitle className="text-left">{hit.title}</DialogTitle>
                  <DialogDescription className="text-left">
                    {hit.source === "modrinth" ? "Modrinth" : "CurseForge"} ·{" "}
                    {hit.downloads.toLocaleString()} downloads
                  </DialogDescription>
                </div>
              </div>
            </DialogHeader>

            <div className="flex flex-col gap-4">
              <div className="flex flex-wrap gap-1.5">
                {hit.loaders.map((loader) => (
                  <Badge key={loader} className={cn("border", loaderTone(loader))}>
                    {loader}
                  </Badge>
                ))}
                {hit.gameVersions.slice(0, 3).map((version) => (
                  <Badge key={version} variant="outline">
                    {version}
                  </Badge>
                ))}
                {(project?.categories ?? hit.categories).slice(0, 4).map((category) => (
                  <Badge key={category} variant="outline">
                    {category}
                  </Badge>
                ))}
              </div>

              <p className="text-muted-foreground whitespace-pre-line text-xs leading-relaxed">
                {project?.description || hit.description}
              </p>

              <section className="flex flex-col gap-2">
                <h4 className="flex items-center gap-1.5 text-sm font-semibold">
                  <History className="size-3.5" /> Versions
                </h4>
                {versionsLoading ? (
                  <div className="flex flex-col gap-2">
                    {[0, 1, 2].map((index) => (
                      <Skeleton key={index} className="h-12 w-full" />
                    ))}
                  </div>
                ) : (versions ?? []).length === 0 ? (
                  <p className="text-muted-foreground text-xs">
                    No published versions listed.
                  </p>
                ) : (
                  <ul className="flex flex-col gap-1.5">
                    {(versions ?? []).slice(0, 25).map((version: ModVersion) => (
                      <li
                        key={version.id}
                        className="flex items-center gap-3 rounded-lg border border-white/8 bg-black/20 px-3 py-2"
                      >
                        <div className="flex min-w-0 flex-1 flex-col">
                          <span className="truncate text-xs font-medium">
                            {version.versionNumber}
                          </span>
                          <span className="text-muted-foreground text-[11px]">
                            {version.gameVersions.slice(0, 3).join(", ")}
                            {version.gameVersions.length > 3
                              ? ` +${version.gameVersions.length - 3}`
                              : ""}{" "}
                            · {formatBytes(version.fileSize)} ·{" "}
                            {version.publishedAt
                              ? formatRelative(version.publishedAt)
                              : "unknown date"}
                          </span>
                        </div>
                        <Badge variant={version.versionType === "release" ? "success" : "outline"}>
                          {version.versionType}
                        </Badge>
                        <Button
                          size="sm"
                          variant="secondary"
                          disabled={installing}
                          onClick={() => onInstall(version.id)}
                        >
                          Install
                        </Button>
                      </li>
                    ))}
                  </ul>
                )}
              </section>

              <div className="flex justify-end">
                <Button
                  disabled={installing}
                  onClick={() => onInstall(undefined)}
                >
                  Install latest
                </Button>
              </div>
            </div>
          </>
        )}
      </DialogContent>
    </Dialog>
  );
}
