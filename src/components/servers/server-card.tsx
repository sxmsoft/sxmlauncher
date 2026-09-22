import { useState } from "react";

import { Lock, Radio, Signal, Star, Users, Zap } from "lucide-react";

import { Badge, StatusDot } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardFooter } from "@/components/ui/card";
import { Hint } from "@/components/ui/tooltip";
import { cn, loaderTone, pingTone } from "@/lib/utils";
import { freeSlots, isFresh, type ServerListingSummary } from "@/types/server";

/** Decoded icon bytes are large; cache nothing and let the browser dedupe. */
function iconSrc(base64: string | null): string | null {
  if (!base64) return null;
  return base64.startsWith("data:") ? base64 : `data:image/png;base64,${base64}`;
}

/**
 * One world in the browser.
 *
 * Freshness is decided by the heartbeat age, not by the browser's fetch time: a
 * listing whose host stopped heartbeating is dimmed even if it is still in the
 * Redis set for a few more seconds.
 */
export function ServerCard({
  server,
  favorite,
  onToggleFavorite,
  onJoin,
  onPing,
  pinging,
  joining,
  pingMs,
  className,
}: {
  server: ServerListingSummary;
  favorite: boolean;
  onToggleFavorite: (favorite: boolean) => void;
  onJoin: () => void;
  onPing: () => void;
  pinging: boolean;
  joining: boolean;
  pingMs?: number | null;
  className?: string;
}) {
  const [failedIcon, setFailedIcon] = useState(false);
  const src = iconSrc(server.iconBase64);
  const live = isFresh(server);
  const full = freeSlots(server.players) === 0;
  const latency = pingMs ?? server.pingMs;

  return (
    <Card
      className={cn(
        "flex flex-col overflow-hidden transition-all duration-200 hover:border-white/20",
        !live && "opacity-60",
        className,
      )}
    >
      <CardContent className="flex flex-1 flex-col gap-3 p-4">
        <div className="flex items-start gap-3">
          <div className="flex size-11 shrink-0 items-center justify-center overflow-hidden rounded-xl border border-white/10 bg-black/30">
            {src && !failedIcon ? (
              <img
                src={src}
                alt=""
                className="size-full object-cover [image-rendering:pixelated]"
                onError={() => setFailedIcon(true)}
              />
            ) : (
              <span className="text-sm font-semibold">{server.name.slice(0, 2)}</span>
            )}
          </div>

          <div className="flex min-w-0 flex-1 flex-col gap-0.5">
            <span className="flex items-center gap-2">
              <span className="truncate text-sm font-semibold">{server.name}</span>
              {live ? <StatusDot tone="success" pulse /> : <StatusDot tone="muted" />}
            </span>
            <span className="text-muted-foreground truncate text-[11px]">
              hosted by {server.ownerName}
              {server.region ? ` · ${server.region}` : ""}
            </span>
          </div>

          <Button
            variant="ghost"
            size="icon-sm"
            onClick={() => onToggleFavorite(!favorite)}
            aria-label={favorite ? "Remove from favourites" : "Add to favourites"}
          >
            <Star className={cn("size-3.5", favorite && "fill-[var(--warning)] text-[var(--warning)]")} />
          </Button>
        </div>

        <p className="text-muted-foreground line-clamp-2 text-xs leading-relaxed">
          {server.description || "No description"}
        </p>

        <div className="flex flex-wrap items-center gap-1.5">
          <Badge variant="outline">{server.gameVersion}</Badge>
          <Badge className={cn("border", loaderTone(server.loader))}>{server.loader}</Badge>
          {server.modpackName ? <Badge variant="primary">{server.modpackName}</Badge> : null}
          {server.versionMismatch ? <Badge variant="warning">version mismatch</Badge> : null}
          {server.passwordProtected ? (
            <Badge variant="outline">
              <Lock className="size-3" /> locked
            </Badge>
          ) : null}
          {server.tags.slice(0, 2).map((tag) => (
            <Badge key={tag} variant="outline">
              {tag}
            </Badge>
          ))}
        </div>
      </CardContent>

      <CardFooter className="mt-auto justify-between border-t border-white/6 pt-3">
        <div className="flex items-center gap-3 text-[11px]">
          <span className={cn("flex items-center gap-1", full ? "text-[var(--destructive)]" : "text-muted-foreground")}>
            <Users className="size-3" />
            {server.players.online}/{server.players.max}
          </span>
          <Hint
            label={
              server.mode === "relay"
                ? "Relayed session — latency belongs to the relay, not the host"
                : "Measured directly against the host"
            }
          >
            <button
              type="button"
              onClick={onPing}
              className={cn("flex items-center gap-1 tabular-nums", pingTone(latency))}
            >
              <Signal className={cn("size-3", pinging && "animate-pulse")} />
              {pinging ? "…" : latency == null ? "—" : `${latency} ms`}
            </button>
          </Hint>
          <Badge variant={server.mode === "direct_p2p" ? "success" : "outline"}>
            {server.mode === "direct_p2p" ? <Zap className="size-3" /> : <Radio className="size-3" />}
            {server.mode === "direct_p2p" ? "direct" : server.mode}
          </Badge>
        </div>

        <Button size="sm" onClick={onJoin} loading={joining} disabled={!live && full}>
          {full ? "Full" : "Join"}
        </Button>
      </CardFooter>
    </Card>
  );
}
