import { useMemo, useState } from "react";

import { Globe, Link2, RefreshCw, Star } from "lucide-react";

import { PageHeader } from "@/components/common/page-header";
import { JoinCodeDialog } from "@/components/servers/join-code-dialog";
import { MySessions } from "@/components/servers/my-sessions";
import { ServerCard } from "@/components/servers/server-card";
import { ServerFilters } from "@/components/servers/server-filters";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { CardSkeleton, EmptyState, ErrorState } from "@/components/ui/feedback";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import {
  useFavorites,
  useJoinServer,
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
        title="Server browser"
        description="Worlds hosted straight from other players' launchers. Direct P2P when your networks allow it, relayed when they do not."
        actions={
          <>
            <Badge variant={browse.data?.length ? "success" : "outline"}>
              {browse.data?.length ?? 0} live
            </Badge>
            <Button variant="outline" size="sm" onClick={() => void browse.refetch()} loading={browse.isFetching}>
              <RefreshCw className="size-4" /> Refresh
            </Button>
            <Button size="sm" onClick={() => setJoinOpen(true)}>
              <Link2 className="size-4" /> Join with code
            </Button>
          </>
        }
      />

      <ServerFilters filter={filter} onChange={setFilter} gameVersions={gameVersions} />

      {browse.isError ? (
        <ErrorState
          title="Directory unreachable"
          message={
            browse.error instanceof Error
              ? browse.error.message
              : "The Redis directory could not be reached. Check the URL in Settings → Network."
          }
          onRetry={() => void browse.refetch()}
        />
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
