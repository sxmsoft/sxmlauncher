import { PageHeader } from "@/components/common/page-header";
import { ModBrowser } from "@/components/mods/mod-browser";

/**
 * Modpacks.
 *
 * Installing a modpack always creates a **new** Ready instance (game version +
 * loader + mods from the pack). There is no target-instance picker here — that
 * belongs on the per-instance “browse mods” flow.
 */
export function ModpacksPage() {
  return (
    <div className="flex flex-col gap-5">
      <PageHeader
        title="Modpacks"
        description="Search Modrinth and CurseForge. Each pack install creates its own instance with the right loader and mods — no empty instance required."
      />

      <ModBrowser kind="modpack" />
    </div>
  );
}
