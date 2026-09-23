import { useEffect, useState } from "react";

import { Link } from "react-router-dom";

import { useTranslation } from "react-i18next";

import { PageHeader } from "@/components/common/page-header";
import { ModBrowser } from "@/components/mods/mod-browser";
import { Badge } from "@/components/ui/badge";
import { Select } from "@/components/ui/input";
import { useInstances } from "@/hooks/queries";
import { useUiStore } from "@/stores/ui";

/**
 * Modpacks & mods.
 *
 * The target instance is chosen at the top and then threads through everything:
 * search results are filtered to its game version and loader, and installs land
 * in it. Installing a *modpack* creates its own instance instead — a pack is a
 * complete environment, not a set of files to merge.
 */
export function ModpacksPage() {
  const { t } = useTranslation();
  const instances = useInstances();
  const selectedId = useUiStore((state) => state.selectedInstanceId);
  const select = useUiStore((state) => state.selectInstance);
  const [targetId, setTargetId] = useState<string | null>(selectedId);

  // Follow the global selection when the user changes it elsewhere.
  useEffect(() => {
    setTargetId(selectedId);
  }, [selectedId]);

  const target = (instances.data ?? []).find((instance) => instance.id === targetId) ?? null;

  return (
    <div className="flex flex-col gap-5">
      <PageHeader
        title={t("browse.title")}
        description={t("browse.subtitle")}
        actions={
          <div className="flex items-center gap-2">
            <Link to="/custom-packs" className="glass-pill px-3 py-1.5 text-xs font-medium text-[var(--text)]">
              {t("browse.custom")}
            </Link>
            <Badge variant="outline">{t("browse.target")}</Badge>
            <Select
              value={targetId ?? ""}
              onChange={(event) => {
                const next = event.target.value || null;
                setTargetId(next);
                select(next);
              }}
              className="w-56"
            >
              <option value="">{t("browse.choose")}</option>
              {(instances.data ?? []).map((instance) => (
                <option key={instance.id} value={instance.id}>
                  {instance.name} · {instance.gameVersion}
                </option>
              ))}
            </Select>
          </div>
        }
      />

      <ModBrowser instance={target} kind="modpack" />
    </div>
  );
}
