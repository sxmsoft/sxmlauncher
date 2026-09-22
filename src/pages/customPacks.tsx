import { useState } from "react";

import { useNavigate } from "react-router-dom";

import { Blocks, Plus, Trash2 } from "lucide-react";

import { PageHeader } from "@/components/common/page-header";
import { ModBrowser } from "@/components/mods/mod-browser";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card } from "@/components/ui/card";
import { CardSkeleton, EmptyState, ErrorState } from "@/components/ui/feedback";
import { Input } from "@/components/ui/input";
import {
  useCustomPackItems,
  useCustomPackRemoveItem,
  useCreateCustomPack,
  useCustomPacks,
  useDeleteCustomPack,
  usePlayCustomPack,
} from "@/hooks/queries";
import { useUiStore } from "@/stores/ui";
import type { CustomPack } from "@/types/modpack";

/** Search results are pinned into the pack being edited (deep-link target). */
function PackBrowserSection({ packId }: { packId: string }) {
  const setAddPackTarget = useUiStore((state) => state.setAddPackTarget);
  return (
    <section className="flex flex-col gap-3">
      <div className="flex items-center justify-between">
        <h2 className="text-sm font-semibold">Add mods</h2>
        <Button variant="ghost" size="sm" onClick={() => setAddPackTarget(null)}>
          Done adding
        </Button>
        <p className="sr-only">Searching {packId}</p>
      </div>
      <ModBrowser kind="mod" />
    </section>
  );
}

/** One pack card in the list. */
const PACK_COVERS = [
  "linear-gradient(135deg, #1a0a2e 0%, #4c1d95 45%, #7c3aed 100%)",
  "linear-gradient(160deg, #0c1a12 0%, #14532d 40%, #22c55e 100%)",
  "linear-gradient(145deg, #1c0a0a 0%, #7f1d1d 40%, #ea580c 100%)",
  "linear-gradient(135deg, #0a1628 0%, #1e3a8a 50%, #38bdf8 100%)",
];

function PackCard({ pack, onOpen }: { pack: CustomPack; onOpen: () => void }) {
  const remove = useDeleteCustomPack();
  const cover = PACK_COVERS[pack.name.length % PACK_COVERS.length];
  return (
    <Card className="overflow-hidden p-0">
      <button type="button" className="block w-full text-left" onClick={onOpen}>
        <div className="relative h-[108px]" style={{ background: cover }}>
          {pack.iconUrl ? <img src={pack.iconUrl} alt="" className="size-full object-cover" /> : null}
          <span className="absolute top-2.5 left-2.5 rounded-md border border-[var(--border)] bg-[color-mix(in_srgb,var(--bg-void)_70%,transparent)] px-2 py-0.5 text-[10px] font-semibold">
            {pack.loader || "Custom"}
          </span>
        </div>
        <div className="p-4">
          <div className="truncate text-sm font-semibold">{pack.name}</div>
          <p className="mt-1 line-clamp-2 min-h-9 text-xs text-[var(--text-muted)]">
            {pack.gameVersion ? `${pack.gameVersion} · your pinned mods` : "Empty — open to pin mods"}
          </p>
          <div className="mt-3 flex items-center justify-between">
            <Badge variant="outline">{pack.gameVersion || "draft"}</Badge>
            <span className="text-[11px] text-[var(--text-faint)]">
              {new Date(pack.updatedAt).toLocaleDateString()}
            </span>
          </div>
        </div>
      </button>
      <div className="flex items-center justify-end gap-1 px-3 pb-3">
        <Button
          variant="ghost"
          size="icon-sm"
          aria-label={`Delete ${pack.name}`}
          loading={remove.isPending}
          onClick={() => remove.mutate(pack.id)}
        >
          <Trash2 className="size-3.5" />
        </Button>
        <Button size="sm" onClick={onOpen}>
          Open
        </Button>
      </div>
    </Card>
  );
}

/**
 * Player-assembled packs, CurseForge-profile style: create a pack, pin mods
 * into it from the browser below, then resolve + install it as an instance.
 */
export function CustomPacksPage() {
  const navigate = useNavigate();
  const packs = useCustomPacks();
  const create = useCreateCustomPack();
  const play = usePlayCustomPack();
  const setAddPackTarget = useUiStore((state) => state.setAddPackTarget);

  const [adding, setAdding] = useState(false);
  const [name, setName] = useState("");
  const [openId, setOpenId] = useState<string | null>(null);

  const openPack = (id: string) => {
    setOpenId(id);
    setAddPackTarget(id);
  };

  return (
    <div className="flex flex-col gap-5">
      <PageHeader
        title="Custom packs"
        description="Build your own modpack: pin mods from Modrinth or CurseForge, then install the whole set as one instance with dependencies resolved."
        actions={
          adding ? null : (
            <Button size="sm" onClick={() => setAdding(true)}>
              <Plus className="size-3.5" /> New pack
            </Button>
          )
        }
      />

      {adding ? (
        <Card className="flex flex-wrap items-center gap-2 p-3">
          <Input
            autoFocus
            value={name}
            placeholder="Pack name"
            className="w-64"
            onChange={(event) => setName(event.target.value)}
            onKeyDown={(event) => {
              if (event.key === "Enter" && name.trim()) {
                create.mutate({ name: name.trim() });
                setName("");
                setAdding(false);
              }
            }}
          />
          <Button
            size="sm"
            loading={create.isPending}
            disabled={!name.trim()}
            onClick={() => {
              create.mutate({ name: name.trim() });
              setName("");
              setAdding(false);
            }}
          >
            Create
          </Button>
          <Button variant="ghost" size="sm" onClick={() => setAdding(false)}>
            Cancel
          </Button>
        </Card>
      ) : null}

      {packs.isError ? (
        <ErrorState
          title="Could not load packs"
          message={packs.error instanceof Error ? packs.error.message : String(packs.error)}
          onRetry={() => void packs.refetch()}
        />
      ) : packs.isLoading ? (
        <div className="grid gap-3">
          {[0, 1, 2].map((index) => (
            <CardSkeleton key={index} />
          ))}
        </div>
      ) : (packs.data?.length ?? 0) === 0 ? (
        <EmptyState
          icon={<Blocks />}
          title="No custom packs yet"
          description="Create a pack, pin mods into it from the browser, and launch it like any other instance."
        />
      ) : (
        <div className="grid gap-4 sm:grid-cols-2 xl:grid-cols-3">
          {(packs.data ?? []).map((pack) => (
            <PackCard key={pack.id} pack={pack} onOpen={() => openPack(pack.id)} />
          ))}
        </div>
      )}

      {openId ? <PackDetail packId={openId} onPlay={(id) => navigate(`/instances/${id}`)} /> : null}

      {openId ? (
        <PackBrowserSection packId={openId} />
      ) : (
        <p className="text-muted-foreground text-[11px]">
          Tip: open a pack to pin mods into it — the browser below filters by the pack's game
          version and loader once the first mod sets them.
        </p>
      )}

      {play.isPending ? (
        <Card className="p-3 text-xs">Resolving and installing pack contents…</Card>
      ) : null}
    </div>
  );
}

/** Detail strip: pinned mods + play button for the open pack. */
function PackDetail({ packId, onPlay }: { packId: string; onPlay: (instanceId: string) => void }) {
  const items = useCustomPackItems(packId);
  const play = usePlayCustomPack();
  const removeItem = useCustomPackRemoveItem(packId);

  return (
    <Card className="flex flex-col gap-2 p-4">
      <div className="flex items-center justify-between">
        <span className="text-sm font-semibold">Pinned mods ({items.data?.length ?? 0})</span>
        <Button
          size="sm"
          loading={play.isPending}
          disabled={(items.data?.length ?? 0) === 0}
          onClick={() =>
            play.mutate(packId, {
              onSuccess: (instance) => onPlay(instance.id),
            })
          }
        >
          Resolve & play
        </Button>
      </div>
      {items.isError ? (
        <span className="text-xs text-[var(--danger)]">
          {items.error instanceof Error ? items.error.message : String(items.error)}
        </span>
      ) : (items.data?.length ?? 0) === 0 ? (
        <span className="text-muted-foreground text-xs">
          Nothing pinned yet — use the search below to add the first mod.
        </span>
      ) : (
        <ul className="grid gap-1">
          {(items.data ?? []).map((item) => (
            <li
              key={`${item.source}-${item.projectId}`}
              className="text-muted-foreground flex items-center gap-2 text-xs"
            >
              <Badge variant="outline">{item.source}</Badge>
              <span className="truncate font-mono">{item.projectId}</span>
              {item.versionId ? (
                <span className="text-[10px]">pinned version</span>
              ) : (
                <span className="text-[10px]">latest</span>
              )}
              <button
                className="ml-auto text-[10px] underline underline-offset-2 hover:text-[var(--danger)]"
                disabled={removeItem.isPending}
                onClick={() => removeItem.mutate(item.projectId)}
              >
                remove
              </button>
            </li>
          ))}
        </ul>
      )}
    </Card>
  );
}
