import { useMemo, useState } from "react";

import { Globe, Link2, Radio, RefreshCw, Star } from "lucide-react";

import { PageHeader } from "@/components/common/page-header";
import { JoinCodeDialog } from "@/components/servers/join-code-dialog";
import { MySessions } from "@/components/servers/my-sessions";
import { ServerCard } from "@/components/servers/server-card";
import { ServerFilters } from "@/components/servers/server-filters";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card } from "@/components/ui/card";
import { CardSkeleton, EmptyState } from "@/components/ui/feedback";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import {
  useFavorites,
  useJoinServer,
  useLanWorlds,
  useServerBrowse,
  useServerPing,
  useSetFavorite,
} from "@/hooks/queries";
import { useDebouncedValue } from "@/hooks/use-debounced-value";
import { formatRelative } from "@/lib/utils";
import {
  freeSlots,
  summarizeListing,
  type ServerFilter,
  type ServerListingSummary,
} from "@/types/server";

/**
 * The global P2P server browser.
 *
 * Listings come from the Redis directory and expire on their own heartbeat, so
 * there is nothing to clean up and no stale worlds: a host that closed is gone a
 * few seconds later. Favourites are cached locally so links to friends' worlds
 * survive a restart even when the directory does not.
 */
export function ServersPage() {
  const [filter, setFilter] = useState<ServerFilter>({ limit: 60 });
  const [joinOpen, setJoinOpen] = useState(false);
  const [pinged, setPinged] = useState<Record<string, number | null>>({});

  // Debounce only the free-text part; toggling filters should feel instant.
  const debouncedQuery = useDebouncedValue(filter.query ?? "", 350);
  const query: ServerFilter = useMemo(
    () => ({ ...filter, query: debouncedQuery || undefined }),
    [filter, debouncedQuery],
  );

  const browse = useServerBrowse(query);
  const lan = useLanWorlds();
  const favorites = useFavorites();
  const setFavorite = useSetFavorite();
  const ping = useServerPing();
  const join = useJoinServer();

  const favoriteIds = new Set((favorites.data ?? []).map((entry) => entry.listing.id));

  // Game versions offered in the filter bar come from what is actually live.
  const gameVersions = useMemo(() => {
    const versions = new Set<string>();
    for (const server of browse.data ?? []) versions.add(server.gameVersion);
    return [...versions].sort().reverse();
  }, [browse.data]);

  const servers = useMemo(() => {
    const list = browse.data ?? [];
    return list.filter((server) => {
      if (filter.hideFull && freeSlots(server.players) === 0) return false;
      if (filter.hidePasswordProtected && server.passwordProtected) return false;
      if (filter.maxPingMs != null) {
        const latency = pinged[server.id] ?? server.pingMs;
        if (latency != null && latency > filter.maxPingMs) return false;
      }
      return true;
    });
  }, [browse.data, filter.hideFull, filter.hidePasswordProtected, filter.maxPingMs, pinged]);

  const pingServer = (id: string) =>
    ping.mutate(id, {
      onSuccess: (latency) => setPinged((previous) => ({ ...previous, [id]: latency })),
    });

  const renderGrid = (list: ServerListingSummary[]) => (
    <div className="grid gap-3 sm:grid-cols-2 2xl:grid-cols-3">
      {list.map((server) => (
        <ServerCard
          key={server.id}
          server={server}
          favorite={favoriteIds.has(server.id)}
          pingMs={pinged[server.id] ?? server.pingMs}
          pinging={ping.isPending && ping.variables === server.id}
          joining={join.isPending && join.variables === server.id}
          onToggleFavorite={(favorite) => setFavorite.mutate({ id: server.id, favorite })}
          onPing={() => pingServer(server.id)}
          onJoin={() => join.mutate(server.id)}
        />
      ))}
    </div>
  );

  // Cached favourites are full listings; the card wants the browser's summary
  // shape, so they are normalized the same way the directory does it.
  const cachedFavorites = (favorites.data ?? []).map((entry) => summarizeListing(entry.listing));

  return (
    <div className="flex flex-col gap-5">
      <PageHeader
        title="Host"
        description="Share a world with friends — P2P directory, LAN, or a join code."
        actions={
          <>
            <Button variant="outline" size="sm" onClick={() => void browse.refetch()} loading={browse.isFetching}>
              <RefreshCw className="size-4" /> Refresh
            </Button>
            <Button size="sm" onClick={() => setJoinOpen(true)}>
              <Link2 className="size-4" /> Join with code
            </Button>
          </>
        }
      />

      <Card className="flex flex-col items-center px-8 py-12 text-center">
        <div className="mb-5 grid size-[72px] place-items-center rounded-[20px] border border-[var(--border)] bg-[var(--surface-2)] shadow-[0_0_40px_var(--accent-dim)]">
          <Radio className="size-8 text-[var(--accent-soft)]" strokeWidth={1.5} />
        </div>
        <h2 className="text-xl font-semibold">Host coming soon</h2>
        <p className="mt-2 max-w-sm text-[13px] text-[var(--text-muted)]">
          Global P2P is paused in this build. The screen stays here so it does not look broken —
          LAN worlds and join codes still work.
        </p>
        <div className="mt-4 mb-6 inline-flex items-center gap-2 rounded-full border border-[rgba(251,191,36,0.25)] bg-[rgba(251,191,36,0.1)] px-3.5 py-2 text-xs font-medium text-[var(--warning)]">
          <span className="size-1.5 rounded-full bg-[var(--warning)]" />
          P2P is paused · UI shell is active
        </div>
        <div className="grid w-full max-w-xl grid-cols-3 gap-3 opacity-50">
          <div className="rounded-[var(--radius-md)] border border-dashed border-[var(--border)] bg-[var(--surface-2)] p-4 text-left">
            <div className="text-[11px] text-[var(--text-faint)]">Status</div>
            <div className="mt-1 text-[13px] font-medium text-[var(--text-muted)]">Offline</div>
          </div>
          <div className="rounded-[var(--radius-md)] border border-dashed border-[var(--border)] bg-[var(--surface-2)] p-4 text-left">
            <div className="text-[11px] text-[var(--text-faint)]">Players</div>
            <div className="mt-1 text-[13px] font-medium text-[var(--text-muted)]">— / 8</div>
          </div>
          <div className="rounded-[var(--radius-md)] border border-dashed border-[var(--border)] bg-[var(--surface-2)] p-4 text-left">
            <div className="text-[11px] text-[var(--text-faint)]">Directory</div>
            <div className="mt-1 text-[13px] font-medium text-[var(--text-muted)]">Paused</div>
          </div>
        </div>
      </Card>

      {(lan.data?.length ?? 0) > 0 ? (
        <section className="flex flex-col gap-3">
          <h2 className="text-sm font-semibold">Nearby on LAN</h2>
          <div className="grid gap-3 sm:grid-cols-2 2xl:grid-cols-3">
            {(lan.data ?? []).map((world) => (
              <Card key={world.id} className="p-4">
                <div className="truncate text-sm font-semibold">{world.name}</div>
                <p className="mt-1 truncate text-xs text-[var(--text-muted)]">{world.motd || world.address}</p>
                <div className="mt-3 flex flex-wrap gap-1.5">
                  <Badge variant="outline">{world.gameVersion}</Badge>
                  <Badge variant="outline">{world.loader}</Badge>
                  <span className="font-mono text-[11px] text-[var(--text-faint)]">
                    {world.players}/{world.maxPlayers}
                  </span>
                </div>
              </Card>
            ))}
          </div>
        </section>
      ) : null}

      <ServerFilters filter={filter} onChange={setFilter} gameVersions={gameVersions} />

      {browse.isError ? (
        <p className="text-xs text-[var(--text-muted)]">
          Directory is not reachable
          {browse.error instanceof Error ? ` — ${browse.error.message}` : ""}. Join with a code, or
          look for LAN worlds above.
        </p>
      ) : null}

      <Tabs defaultValue="live">
        <TabsList>
          <TabsTrigger value="live">
            <Globe /> Live worlds
          </TabsTrigger>
          <TabsTrigger value="favorites">
            <Star /> Favourites
          </TabsTrigger>
        </TabsList>

        <TabsContent value="live">
          {browse.isLoading ? (
            <div className="grid gap-3 sm:grid-cols-2 2xl:grid-cols-3">
              {[0, 1, 2, 3, 4, 5].map((index) => (
                <CardSkeleton key={index} />
              ))}
            </div>
          ) : servers.length === 0 ? (
            <EmptyState
              icon={<Globe />}
              title="No worlds match"
              description={
                browse.data && browse.data.length > 0
                  ? "Filters are hiding everything that is live right now. Try widening them."
                  : "Nobody is hosting right now. Start a world from the Play page and it appears here for everyone."
              }
            />
          ) : (
            renderGrid(servers)
          )}
        </TabsContent>

        <TabsContent value="favorites">
          {cachedFavorites.length === 0 ? (
            <EmptyState
              icon={<Star />}
              title="No favourites yet"
              description="Star a world to keep it here — favourites are cached locally, so they survive restarts and work offline."
            />
          ) : (
            <div className="flex flex-col gap-3">
              {renderGrid(cachedFavorites)}
              <p className="text-muted-foreground text-[11px]">
                Cached favourites may be offline. Freshest entry:{" "}
                {formatRelative(
                  (favorites.data ?? [])[0]?.lastSeenAt ?? new Date().toISOString(),
                )}
                .
              </p>
            </div>
          )}
        </TabsContent>
      </Tabs>

      <MySessions />
      <JoinCodeDialog open={joinOpen} onOpenChange={setJoinOpen} />
    </div>
  );
}
