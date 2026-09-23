import { useEffect, useMemo, useState } from "react";

import { useNavigate } from "react-router-dom";

import { FolderSearch, Plus } from "lucide-react";
import { useTranslation } from "react-i18next";

import { PageHeader } from "@/components/common/page-header";
import { CreateInstanceDialog } from "@/components/instance/create-instance-dialog";
import { InstanceCard } from "@/components/instance/instance-card";
import { Button } from "@/components/ui/button";
import { useImportInstance, useInstances, useSelectedInstance } from "@/hooks/queries";
import { cn } from "@/lib/utils";
import { useUiStore } from "@/stores/ui";
import type { LoaderKind } from "@/types/instance";

const FILTERS: Array<{ id: "all" | LoaderKind | "modded"; labelKey: string }> = [
  { id: "all", labelKey: "library.filters.all" },
  { id: "vanilla", labelKey: "library.filters.vanilla" },
  { id: "fabric", labelKey: "library.filters.fabric" },
  { id: "forge", labelKey: "library.filters.forge" },
  { id: "modded", labelKey: "library.filters.modded" },
];

/** Installed instances. Selecting one is what Home launches. */
export function LibraryPage() {
  const { t } = useTranslation();
  const navigate = useNavigate();
  const instances = useInstances();
  const selected = useSelectedInstance();
  const select = useUiStore((state) => state.selectInstance);
  const query = useUiStore((state) => state.chromeQuery);
  const importFolder = useImportInstance();

  const [createOpen, setCreateOpen] = useState(false);
  useEffect(() => {
    const open = () => setCreateOpen(true);
    window.addEventListener("sxml-new-instance", open);
    return () => window.removeEventListener("sxml-new-instance", open);
  }, []);
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
    <div className="flex flex-col gap-5" data-context="library">
      <PageHeader
        title={t("library.title")}
        description={t("library.subtitle")}
        actions={
          <>
            <Button
              variant="outline"
              size="sm"
              className="rounded-full"
              onClick={() => importFolder.mutate()}
              loading={importFolder.isPending}
            >
              <FolderSearch className="size-4" /> {t("library.import")}
            </Button>
            <Button size="sm" className="rounded-full" onClick={() => setCreateOpen(true)}>
              <Plus className="size-4" /> {t("library.new")}
            </Button>
          </>
        }
      />

      <div className="flex flex-wrap gap-1.5">
        {FILTERS.map((entry) => (
          <button
            key={entry.id}
            type="button"
            onClick={() => setFilter(entry.id)}
            className={cn(
              "h-8 rounded-full px-3 text-xs font-medium text-[var(--text-muted)]",
              filter === entry.id
                ? "bg-[var(--accent)] text-white"
                : "glass-pill hover:text-[var(--text)]",
            )}
          >
            {t(entry.labelKey)}
          </button>
        ))}
      </div>

      {visible.length === 0 ? (
        <p className="text-sm text-[var(--text-muted)]">{t("library.empty")}</p>
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
        <div className="glass flex flex-wrap items-center justify-between gap-3 rounded-full px-4 py-3">
          <div className="flex items-center gap-3 text-[13px] text-[var(--text-muted)]">
            <span className="inline-flex h-[22px] items-center rounded-full bg-[var(--accent-dim)] px-2 font-mono text-[11px] text-[var(--accent-soft)]">
              {t("library.selected")}
            </span>
            <span>
              <strong className="font-semibold text-[var(--text)]">{selected.name}</strong>
              {" · "}
              {selected.gameVersion} {selected.loader.kind} · {selected.modCount} {t("home.mods").toLowerCase()}
            </span>
          </div>
          <Button className="rounded-full" onClick={() => navigate("/")}>
            {t("home.play")}
          </Button>
        </div>
      ) : null}

      <CreateInstanceDialog open={createOpen} onOpenChange={setCreateOpen} />
    </div>
  );
}
