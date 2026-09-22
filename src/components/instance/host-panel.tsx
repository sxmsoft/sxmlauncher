import { useState } from "react";

import { Ban, Copy, Radio, RadioTower, Zap } from "lucide-react";

import { Badge, StatusDot } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { ConfirmDialog } from "@/components/ui/dialog";
import { Switch, NumberInput } from "@/components/ui/switch";
import { Input } from "@/components/ui/input";
import { Hint } from "@/components/ui/tooltip";
import { useHostWorld, useKickGuest, useSettings, useStopHost } from "@/hooks/queries";
import { cn, copyToClipboard } from "@/lib/utils";
import { useSessionsStore } from "@/stores/sessions";
import { toast } from "@/stores/ui";
import type { Instance } from "@/types/instance";
import type { HostStatus } from "@/types/server";

/**
 * "Host World to Friends".
 *
 * One switch does everything the player cares about: bind a local bridge to the
 * world, punch a hole through NAT (or fall back to the relay), publish the
 * listing to the Redis directory, and start heartbeating so the browser drops it
 * the moment the app stops.
 *
 * The panel then keeps the player oriented: whether the connection is direct or
 * relayed, the code to share, and who is currently connected.
 */
export function HostPanel({ instance }: { instance: Instance | null }) {
  const { data: settings } = useSettings();
  const host = useSessionsStore((state) => state.hostOfInstance(instance?.id ?? null));

  const [name, setName] = useState(instance?.name ?? "My world");
  const [maxPlayers, setMaxPlayers] = useState(settings?.maxHostedPlayers ?? 8);
  const [password, setPassword] = useState("");
  const [isPublic, setIsPublic] = useState(settings?.shareByDefault ?? true);
  const [advanced, setAdvanced] = useState(false);
  const [confirmStop, setConfirmStop] = useState(false);

  const start = useHostWorld();
  const stop = useStopHost();

  const hosting = host != null;

  if (hosting) {
    return (
      <Card>
        <HostingStatus
          status={host}
          onStop={() => setConfirmStop(true)}
          stopping={stop.isPending}
        />
        <ConfirmDialog
          open={confirmStop}
          onOpenChange={setConfirmStop}
          title="Stop hosting?"
          description="Everyone connected to this world will be disconnected and the listing will disappear from the server browser."
          confirmLabel="Stop hosting"
          destructive
          busy={stop.isPending}
          onConfirm={() =>
            stop.mutate(host.id, { onSuccess: () => setConfirmStop(false) })
          }
        />
      </Card>
    );
  }

  return (
    <Card>
      <CardHeader className="flex-row items-center justify-between">
        <div className="flex flex-col gap-1">
          <CardTitle>Host World to Friends</CardTitle>
          <p className="text-muted-foreground text-sm">
            The launcher starts this instance&apos;s server, punches a hole (or uses the
            relay), and gives you a join code. No “Open to LAN” step required.
          </p>
        </div>
        <Switch
          checked={false}
          onCheckedChange={() =>
            start.mutate({
              instanceId: instance?.id,
              name: name.trim() || instance?.name || "My world",
              description: instance?.description ?? "",
              maxPlayers,
              password: password || undefined,
              public: isPublic,
              worldName: instance?.name,
            })
          }
          disabled={start.isPending}
          aria-label="Start hosting"
        />
      </CardHeader>

      <CardContent className="flex flex-col gap-4">
        <div className="grid gap-3 sm:grid-cols-2">
          <div className="flex flex-col gap-1.5">
            <span className="text-muted-foreground text-[10px] font-medium tracking-widest uppercase">
              Session name
            </span>
            <Input value={name} onChange={(event) => setName(event.target.value)} maxLength={48} />
          </div>
          <div className="flex flex-col gap-1.5">
            <span className="text-muted-foreground text-[10px] font-medium tracking-widest uppercase">
              Max players
            </span>
            <NumberInput
              value={maxPlayers}
              min={1}
              max={64}
              onValueChange={setMaxPlayers}
              suffix="players"
            />
          </div>
        </div>

        {advanced ? (
          <div className="grid gap-3 sm:grid-cols-2">
            <div className="flex flex-col gap-1.5">
              <span className="text-muted-foreground text-[10px] font-medium tracking-widest uppercase">
                Password (optional)
              </span>
              <Input
                type="password"
                value={password}
                onChange={(event) => setPassword(event.target.value)}
                placeholder="leave empty for open"
              />
            </div>
            <label className="flex items-center gap-3 self-end pb-1 text-sm">
              <Switch checked={isPublic} onCheckedChange={setIsPublic} />
              List in the global browser
            </label>
          </div>
        ) : null}

        <div className="flex items-center justify-between gap-3">
          <Button variant="ghost" size="sm" onClick={() => setAdvanced(!advanced)}>
            {advanced ? "Hide options" : "Session options"}
          </Button>
          <span className="text-muted-foreground text-xs">
            {isPublic ? "Visible to everyone in the browser" : "Share-code only"}
          </span>
        </div>
      </CardContent>
    </Card>
  );
}

/** Live view of a hosted world: mode, share code, guests. */
function HostingStatus({
  status,
  onStop,
  stopping,
}: {
  status: HostStatus;
  onStop: () => void;
  stopping: boolean;
}) {
  const kick = useKickGuest();
  const [copied, setCopied] = useState(false);

  const direct = status.mode === "direct_p2p";

  return (
    <>
      <CardHeader className="flex-row items-start justify-between gap-4">
        <div className="flex flex-col gap-1">
          <CardTitle className="flex items-center gap-2">
            <StatusDot tone={direct ? "success" : "warning"} pulse />
            Hosting {status.summary.name}
          </CardTitle>
          <p className="text-muted-foreground text-sm">{status.natAdvice}</p>
        </div>
        <Badge variant={direct ? "success" : "warning"}>
          {direct ? <Zap className="size-3" /> : <RadioTower className="size-3" />}
          {direct ? "direct P2P" : "relay"}
        </Badge>
      </CardHeader>

      <CardContent className="flex flex-col gap-4">
        <div className="flex flex-wrap items-center gap-2">
          <div className="flex items-center gap-2 rounded-lg border border-white/10 bg-black/30 px-3 py-2">
            <Radio className="text-muted-foreground size-4" />
            <code className="text-sm tracking-[0.15em] tabular-nums">{status.shareCode}</code>
          </div>
          <Button
            variant="outline"
            size="sm"
            onClick={() => {
              void copyToClipboard(status.shareCode).then((ok) => {
                setCopied(ok);
                if (ok) {
                  toast.success("Share code copied");
                  window.setTimeout(() => setCopied(false), 2000);
                }
              });
            }}
          >
            <Copy className="size-3.5" />
            {copied ? "Copied" : "Copy code"}
          </Button>
          {status.publicEndpoint ? (
            <Hint label="Public endpoint other peers dial (STUN discovery)">
              <Badge variant="outline">{status.publicEndpoint}</Badge>
            </Hint>
          ) : null}
          <Button variant="destructive" size="sm" className="ml-auto" onClick={onStop} loading={stopping}>
            Stop hosting
          </Button>
        </div>

        <div className="flex flex-col gap-2">
          <span className="text-muted-foreground text-[10px] font-medium tracking-widest uppercase">
            Players ({status.summary.players.online}/{status.summary.players.max})
          </span>
          {status.guests.length === 0 ? (
            <p className="text-muted-foreground text-xs">
              Nobody has joined yet. Friends can paste the code or find the world in the
              browser.
            </p>
          ) : (
            <ul className="flex flex-col gap-1.5">
              {status.guests.map((guest) => (
                <li
                  key={guest.peerId}
                  className={cn(
                    "flex items-center gap-3 rounded-lg border border-white/8 bg-black/20 px-3 py-2",
                  )}
                >
                  <StatusDot tone="success" />
                  <span className="text-sm font-medium">{guest.username ?? "connecting…"}</span>
                  <span className="text-muted-foreground text-xs">
                    {guest.protocolVersion ? `protocol ${guest.protocolVersion}` : guest.address}
                  </span>
                  <Button
                    variant="ghost"
                    size="sm"
                    className="ml-auto hover:text-[var(--destructive)]"
                    onClick={() => kick.mutate({ id: status.id, peerId: guest.peerId })}
                  >
                    <Ban className="size-3.5" /> Remove
                  </Button>
                </li>
              ))}
            </ul>
          )}
        </div>
      </CardContent>
    </>
  );
}
