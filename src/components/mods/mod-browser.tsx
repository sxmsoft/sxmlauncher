import { useEffect, useState } from "react";

import { Box, Download, ListFilter, Package, Plus, Search, TriangleAlert } from "lucide-react";
import { useTranslation } from "react-i18next";

import { ModCard } from "@/components/mods/mod-card";
import { ModpackDetailDialog } from "@/components/mods/modpack-detail-dialog";
import { ModVersionDialog } from "@/components/mods/mod-version-dialog";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card } from "@/components/ui/card";
import { CardSkeleton, EmptyState, ErrorState } from "@/components/ui/feedback";
import { Hint } from "@/components/ui/tooltip";
import { Input, Select } from "@/components/ui/input";
import { Tabs, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { useDebouncedValue } from "@/hooks/use-debounced-value";
import {
  useAppInfo,
  useCustomPackAddItem,
  useCustomPacks,
  useDownloadMod,
  useInstallMods,
  useInstallModpack,
  useInstances,
  useModSearch,
} from "@/hooks/queries";
import { useUiStore, toast } from "@/stores/ui";
import type { Instance } from "@/types/instance";
import type { ModSearchHit, ModSource, ModVersion } from "@/types/modpack";

const SORTS = [
  { value: "relevance", label: "Relevance" },
  { value: "downloads", label: "Downloads" },
  { value: "follows", label: "Followers" },
  { value: "newest", label: "Newest" },
  { value: "updated", label: "Recently updated" },
];

const PAGE_SIZE = 24;
const MAX_PAGE_BUTTONS = 7;

/** Numbered pagination with ellipsis, e.g. 1 … 4 5 6 … 23. */
function pageButtons(current: number, total: number): (number | "…")[] {
  if (total <= MAX_PAGE_BUTTONS) {
    return Array.from({ length: total }, (_, index) => index + 1);
  }
  const pages: (number | "…")[] = [1];
  const start = Math.max(2, current - 1);
  const end = Math.min(total - 1, current + 1);
  if (start > 2) pages.push("…");
  for (let page = start; page <= end; page += 1) pages.push(page);
  if (end < total - 1) pages.push("…");
  pages.push(total);
  return pages;
}

function Pagination({
  page,
  totalPages,
  onChange,
}: {
  page: number;
  totalPages: number;
  onChange: (page: number) => void;
}) {
  if (totalPages <= 1) return null;
  return (
    <nav className="flex items-center justify-center gap-1 pt-4" aria-label="Search pages">
      <Button
        variant="ghost"
        size="sm"
        disabled={page <= 1}
        onClick={() => onChange(page - 1)}
      >
        Prev
      </Button>
      {pageButtons(page, totalPages).map((entry, index) =>
        entry === "…" ? (
          <span key={`gap-${index}`} className="text-muted-foreground px-1.5 text-sm">
            …
          </span>
        ) : (
          <Button
            key={entry}
            variant={entry === page ? "default" : "ghost"}
            size="sm"
            className="min-w-8 tabular-nums"
            aria-current={entry === page ? "page" : undefined}
            onClick={() => onChange(entry)}
          >
            {entry}
          </Button>
        ),
      )}
      <Button
        variant="ghost"
        size="sm"
        disabled={page >= totalPages}
        onClick={() => onChange(page + 1)}
      >
        Next
      </Button>
    </nav>
  );
}

/**
 * Unified Modrinth / CurseForge browser.
 *
 * Two axes the player cares about: which registry, and mods vs modpacks.
 * Modpacks always create their own instance (that is what a pack *is*); plain
 * mods install into the instance you are browsing from, together with whatever
 * dependencies the resolver finds — or download on their own via the second
 * button, or get pinned into a custom pack.
 */
export function ModBrowser({
  instance,
  className,
  kind = "mod",
}: {
  instance?: Instance | null;
  className?: string;
  /** Which registry slice to browse: plain mods or whole modpacks. */
  kind?: "mod" | "modpack";
}) {
  const [source, setSource] = useState<ModSource>("modrinth");
  const [query, setQuery] = useState("");
  const [loader, setLoader] = useState(instance?.loader.kind ?? "");
  const [sort, setSort] = useState("relevance");
  const [page, setPage] = useState(1);
  const [versionPick, setVersionPick] = useState<ModSearchHit | null>(null);
  const [detailPick, setDetailPick] = useState<ModSearchHit | null>(null);

  const debouncedQuery = useDebouncedValue(query, 400);
  const { data: appInfo } = useAppInfo();
  const selectedId = useUiStore((state) => state.selectedInstanceId);
  const addPackTargetId = useUiStore((state) => state.addPackTargetId);
  const setAddPackTarget = useUiStore((state) => state.setAddPackTarget);
  const hasInstances = (instance != null || selectedId != null) && !addPackTargetId;
  const targetId = instance?.id ?? selectedId ?? null;

  const { t } = useTranslation();
  const instances = useInstances();
  const installMods = useInstallMods(instance?.id ?? selectedId);
  const installPack = useInstallModpack();
  const downloadMod = useDownloadMod();
  const packs = useCustomPacks();
  const targetPack = (packs.data ?? []).find((pack) => pack.id === addPackTargetId) ?? null;
  const addToPack = useCustomPackAddItem(addPackTargetId ?? "");

  // Typing, changing filters or switching registries restarts pagination.
  useEffect(() => {
    setPage(1);
  }, [debouncedQuery, loader, sort, source]);

  const search = useModSearch({
    query: debouncedQuery || undefined,
    source,
    // The backend defaults a missing type to "mod", so the page's intent must
    // travel explicitly: Modpacks browses packs, Custom-packs browse mods.
    projectType: kind,
    gameVersion: kind === "mod" ? instance?.gameVersion : undefined,
    loader: loader || undefined,
    sort,
    index: (page - 1) * PAGE_SIZE,
    limit: PAGE_SIZE,
  });

  const hits = search.data?.hits ?? [];
  // The registries report matching totals, but never trust them blindly.
  const total = search.data?.total ?? 0;
  const totalPages = Math.max(1, Math.ceil(total / PAGE_SIZE));
  const safePage = Math.min(page, totalPages);

  const curseforgeBlocked =
    source === "curseforge" && appInfo != null && !appInfo.curseforgeConfigured;
  const targetName = instance?.name ?? "the selected instance";

  const libraryKey = (source: string, projectId: string) => `${source}:${projectId}`;
  const installedPacks = new Set(
    (instances.data ?? []).flatMap((entry) =>
      entry.sourcePack ? [libraryKey(entry.sourcePack.source, entry.sourcePack.projectId)] : [],
    ),
  );
  const packInLibrary = (hit: ModSearchHit) =>
    hit.projectType === "modpack" && installedPacks.has(libraryKey(hit.source, hit.id));

  const installHit = (hit: ModSearchHit) => {
    if (packInLibrary(hit)) return;
    if (hit.projectType === "modpack") {
      // Packs always create their own instance — never require a pre-selected one.
      installPack.mutate({ projectId: hit.id, name: hit.title, source: hit.source });
      return;
    }
    if (addPackTargetId) {
      addToPack.mutate({ source: hit.source, projectId: hit.id });
      toast.success(`Added ${hit.title}`, `Pin more mods or play the pack from Custom Packs.`);
      return;
    }
    if (!targetId) {
      toast.warning(
        "No instance selected",
        "Create an instance first, then install mods into it. Modpacks create an instance automatically.",
      );
      return;
    }
    installMods.mutate([{ source: hit.source, projectId: hit.id }]);
  };

  const renderActions = (hit: ModSearchHit) => (
    <>
      {hit.projectType !== "modpack" && targetId ? (
        <Hint label="Download just this mod">
          <Button
            variant="ghost"
            size="icon-sm"
            disabled={downloadMod.isPending}
            onClick={() =>
              downloadMod.mutate({
                instanceId: targetId,
                source: hit.source,
                projectId: hit.id,
              })
            }
          >
            <Download className="size-3.5" />
            <span className="sr-only">Download</span>
          </Button>
        </Hint>
      ) : null}
      {hit.projectType !== "modpack" && targetPack ? (
        <Hint label={`Pin into “${targetPack.name}”`}>
          <Button
            variant="ghost"
            size="icon-sm"
            disabled={addToPack.isPending}
            onClick={() => {
              addToPack.mutate({ source: hit.source, projectId: hit.id });
              toast.success(`Pinned ${hit.title}`, `Continue in Custom Packs.`);
            }}
          >
            <Plus className="size-3.5" />
            <span className="sr-only">Add to pack</span>
          </Button>
        </Hint>
      ) : null}
    </>
  );

  const actionLabel = addPackTargetId
    ? "Pin"
    : hits.some((hit) => hit.projectType === "modpack")
      ? "Install pack"
      : "Install";

  return (
    <div className={className}>
      <div className="flex flex-wrap items-center gap-2">
        <Tabs value={source} onValueChange={(value) => setSource(value as ModSource)}>
          <TabsList>
            <TabsTrigger value="modrinth">Modrinth</TabsTrigger>
            <TabsTrigger value="curseforge">CurseForge</TabsTrigger>
          </TabsList>
        </Tabs>

        <div className="relative min-w-64 flex-1">
          <Search className="text-muted-foreground pointer-events-none absolute top-1/2 left-3 size-4 -translate-y-1/2" />
          <Input
            value={query}
            onChange={(event) => setQuery(event.target.value)}
            placeholder="Search mods and modpacks…"
            className="pl-9"
          />
        </div>

        <Select value={loader} onChange={(event) => setLoader(event.target.value)} className="w-36">
          <option value="">Any loader</option>
          <option value="fabric">Fabric</option>
          <option value="quilt">Quilt</option>
          <option value="forge">Forge</option>
          <option value="neoforge">NeoForge</option>
        </Select>

        <Select value={sort} onChange={(event) => setSort(event.target.value)} className="w-44">
          {SORTS.map((option) => (
            <option key={option.value} value={option.value}>
              {option.label}
            </option>
          ))}
        </Select>
      </div>

      <div className="mt-3 flex flex-wrap items-center gap-2 text-xs">
        {targetPack ? (
          <Badge variant="primary" className="gap-1">
            <Box className="size-3" />
            pinning into {targetPack.name}
            <button
              className="ml-1 underline underline-offset-2"
              onClick={() => setAddPackTarget(null)}
            >
              cancel
            </button>
          </Badge>
        ) : kind === "modpack" ? (
          <Badge variant="outline">{t("browse.packOwnInstance")}</Badge>
        ) : instance ? (
          <Badge variant="primary">
            installing into {instance.name} · {instance.gameVersion}
          </Badge>
        ) : (
          <Badge variant="warning">no instance selected — modpacks will create one</Badge>
        )}
        {search.data ? (
          <span className="text-muted-foreground">
            {total.toLocaleString()} results · page {safePage}/{totalPages}
          </span>
        ) : null}
        <span className="text-muted-foreground flex items-center gap-1">
          <ListFilter className="size-3" />
          {sort}
        </span>
      </div>

      {curseforgeBlocked ? (
        <Card className="mt-4 flex items-start gap-3 p-4">
          <TriangleAlert className="mt-0.5 size-4 text-[var(--warning)]" />
          <div className="flex flex-col gap-1">
            <span className="text-sm font-medium">CurseForge needs an API key</span>
            <p className="text-muted-foreground text-xs leading-relaxed">
              Add a CurseForge API key in Settings → General to search and download from
              CurseForge. Modrinth works without any key.
            </p>
          </div>
        </Card>
      ) : null}

      <div className="mt-4">
        {search.isError ? (
          <ErrorState
            title="Search failed"
            message={search.error instanceof Error ? search.error.message : String(search.error)}
            onRetry={() => void search.refetch()}
          />
        ) : search.isLoading ? (
          <div className="grid gap-3 sm:grid-cols-2 xl:grid-cols-3">
            {[0, 1, 2, 3, 4, 5].map((index) => (
              <CardSkeleton key={index} />
            ))}
          </div>
        ) : hits.length === 0 ? (
          <EmptyState
            icon={<Package />}
            title="Nothing found"
            description={
              debouncedQuery
                ? `No projects matched “${debouncedQuery}” on ${source}.`
                : "Type a project name, or browse the most popular results."
            }
          />
        ) : (
          <>
            <div className="grid gap-3 sm:grid-cols-2 xl:grid-cols-3">                {hits.map((hit) => (
                  <ModCard
                    key={`${hit.source}-${hit.id}`}
                    hit={hit}
                    actionLabel={
                      packInLibrary(hit)
                        ? t("browse.inLibrary")
                        : hit.projectType === "modpack"
                          ? "Install pack"
                          : actionLabel
                    }
                    installed={packInLibrary(hit)}
                    installing={installPack.isPending || installMods.isPending || addToPack.isPending}
                    onInstall={() => installHit(hit)}
                    onPickVersion={() => setVersionPick(hit)}
                    extraActions={renderActions(hit)}
                    onOpenDetails={
                      hit.projectType === "modpack" ? () => setDetailPick(hit) : undefined
                    }
                  />
                ))}
            </div>
            <Pagination
              page={safePage}
              totalPages={totalPages}
              onChange={(next) => {
                setPage(next);
                window.scrollTo({ top: 0, behavior: "smooth" });
              }}
            />
          </>
        )}
      </div>

      <ModpackDetailDialog
        hit={detailPick}
        onClose={() => setDetailPick(null)}
        installing={installPack.isPending}
        onInstall={(versionId) => {
          if (!detailPick) return;
          installPack.mutate({
            projectId: detailPick.id,
            versionId,
            name: detailPick.title,
            source: detailPick.source,
          });
          setDetailPick(null);
        }}
      />

      <ModVersionDialog
        open={versionPick != null}
        onOpenChange={(open) => !open && setVersionPick(null)}
        projectId={versionPick?.id ?? null}
        projectTitle={versionPick?.title ?? ""}
        source={versionPick?.source ?? source}
        gameVersion={instance?.gameVersion ?? "1.21.1"}
        loader={loader === "" ? undefined : loader}
        installing={installMods.isPending || installPack.isPending}
        onPick={(version: ModVersion) => {
          if (!versionPick) return;
          if (versionPick.projectType === "modpack") {
            installPack.mutate({
              projectId: versionPick.id,
              versionId: version.id,
              name: versionPick.title,
              source: versionPick.source,
            });
          } else if (addPackTargetId) {
            addToPack.mutate({
              source: versionPick.source,
              projectId: versionPick.id,
              versionId: version.id,
            });
            toast.success(`Pinned ${versionPick.title}`, "Continue in Custom Packs.");
          } else if (hasInstances) {
            installMods.mutate([
              { source: versionPick.source, projectId: versionPick.id, versionId: version.id },
            ]);
          } else {
            toast.warning(
              "No instance selected",
              "Create an instance first, then install mods into it. Modpacks create an instance automatically.",
            );
          }
          setVersionPick(null);
        }}
      />

      <p className="text-muted-foreground mt-4 text-[11px]">
        {kind === "modpack"
          ? t("browse.packOwnInstance")
          : `Dependencies are resolved before anything is written: incompatible mods are reported instead of silently installed into ${targetName}.`}
      </p>
    </div>
  );
}
