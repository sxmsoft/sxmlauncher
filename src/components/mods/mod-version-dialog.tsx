import { useState } from "react";

import { Download } from "lucide-react";

import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { EmptyState, Skeleton } from "@/components/ui/feedback";
import { useModVersions } from "@/hooks/queries";
import { cn, formatBytes, formatRelative } from "@/lib/utils";
import type { ModSource, ModVersion } from "@/types/modpack";

/**
 * Pick an exact version.
 *
 * "Install the latest compatible build" is the right default, but modpacks in
 * particular need pinning: a friend's world runs one specific pack version, and
 * matching it is the difference between joining and a mod-list mismatch.
 */
export function ModVersionDialog({
  open,
  onOpenChange,
  projectId,
  projectTitle,
  source,
  gameVersion,
  loader,
  onPick,
  installing,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  projectId: string | null;
  projectTitle: string;
  source: ModSource;
  gameVersion: string;
  loader?: string;
  onPick: (version: ModVersion) => void;
  installing: boolean;
}) {
  const { data: versions, isLoading } = useModVersions(
    open ? projectId : null,
    source,
    gameVersion,
    loader,
  );
  const [selected, setSelected] = useState<string | null>(null);

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="max-w-xl">
        <DialogHeader>
          <DialogTitle>{projectTitle} · versions</DialogTitle>
          <DialogDescription>
            Showing builds for Minecraft {gameVersion}
            {loader ? ` on ${loader}` : ""}.
          </DialogDescription>
        </DialogHeader>

        <div className="flex max-h-[52vh] flex-col gap-2 overflow-y-auto pr-1">
          {isLoading ? [0, 1, 2].map((index) => <Skeleton key={index} className="h-14 w-full" />) : null}

          {!isLoading && (versions ?? []).length === 0 ? (
            <EmptyState
              title="No compatible versions"
              description="This project has no build for the selected game version and loader."
            />
          ) : null}

          {(versions ?? []).map((version) => (
            <button
              key={version.id}
              type="button"
              onClick={() => setSelected(version.id)}
              className={cn(
                "flex flex-col gap-1 rounded-lg border p-3 text-left transition-colors",
                selected === version.id
                  ? "border-[color-mix(in_oklab,var(--primary)_55%,transparent)] bg-[color-mix(in_oklab,var(--primary)_12%,transparent)]"
                  : "border-white/8 bg-black/20 hover:bg-white/6",
              )}
            >
              <div className="flex items-center gap-2">
                <span className="text-sm font-medium">{version.name}</span>
                <Badge variant={version.versionType === "release" ? "success" : "outline"}>
                  {version.versionType}
                </Badge>
                <span className="text-muted-foreground ml-auto text-[11px]">
                  {formatBytes(version.fileSize)}
                </span>
              </div>
              <div className="text-muted-foreground flex items-center gap-2 text-[11px]">
                <span>{version.fileName}</span>
                <span>·</span>
                <span>{version.publishedAt ? formatRelative(version.publishedAt) : "unknown date"}</span>
                <span>·</span>
                <span>{version.gameVersions.slice(0, 3).join(", ")}</span>
              </div>
            </button>
          ))}
        </div>

        <div className="mt-4 flex justify-end gap-2">
          <Button variant="ghost" onClick={() => onOpenChange(false)}>
            Cancel
          </Button>
          <Button
            disabled={!selected}
            loading={installing}
            onClick={() => {
              const picked = (versions ?? []).find((version) => version.id === selected);
              if (picked) onPick(picked);
            }}
          >
            <Download /> Install this version
          </Button>
        </div>
      </DialogContent>
    </Dialog>
  );
}
