import { useState } from "react";

import { Copy, LogOut, Radio, RadioTower, Zap } from "lucide-react";

import { Badge, StatusDot } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { EmptyState } from "@/components/ui/feedback";
import { useLeaveSession } from "@/hooks/queries";
import { copyToClipboard } from "@/lib/utils";
import { useSessionsStore } from "@/stores/sessions";
import { toast } from "@/stores/ui";

/**
 * Everything this machine is currently connected to.
 *
 * Both directions in one panel: worlds being hosted (with the code to share) and
 * remote worlds joined (with the local bridge address the game was pointed at),
 * so "why is java connected to something?" always has an answer.
 */
export function MySessions() {
  const hosts = useSessionsStore((state) => state.hosts);
  const guests = useSessionsStore((state) => state.guests);
  const leave = useLeaveSession();
  const [copiedId, setCopiedId] = useState<string | null>(null);

  const hostList = Object.values(hosts);
  const guestList = Object.values(guests);

  if (hostList.length === 0 && guestList.length === 0) {
    return (
      <Card>
        <CardHeader>
          <CardTitle>Sessions</CardTitle>
        </CardHeader>
        <CardContent>
          <EmptyState
            icon={<Radio />}
            title="Not in any session"
            description="Host a world from the Play page, or join one from the browser above."
          />
        </CardContent>
      </Card>
    );
  }

  return (
    <Card>
      <CardHeader>
        <CardTitle>Active sessions</CardTitle>
      </CardHeader>
      <CardContent className="flex flex-col gap-2">
        {hostList.map((host) => (
          <div
            key={host.id}
            className="flex flex-wrap items-center gap-3 rounded-lg border border-white/8 bg-black/20 px-3 py-2"
          >
            <StatusDot tone={host.mode === "direct_p2p" ? "success" : "warning"} pulse />
            <span className="text-sm font-medium">{host.summary.name}</span>
            <Badge variant={host.mode === "direct_p2p" ? "success" : "warning"}>
              {host.mode === "direct_p2p" ? <Zap className="size-3" /> : <RadioTower className="size-3" />}
              {host.mode === "direct_p2p" ? "direct" : "relay"}
            </Badge>
            <code className="text-muted-foreground text-xs tracking-[0.15em]">{host.shareCode}</code>
            <Button
              variant="ghost"
              size="sm"
              className="ml-auto"
              onClick={() =>
                void copyToClipboard(host.shareCode).then((ok) => {
                  if (!ok) return;
                  setCopiedId(host.id);
                  toast.success("Share code copied");
                })
              }
            >
              <Copy className="size-3.5" /> {copiedId === host.id ? "Copied" : "Copy"}
            </Button>
          </div>
        ))}

        {guestList.map((guest) => (
          <div
            key={guest.id}
            className="flex flex-wrap items-center gap-3 rounded-lg border border-white/8 bg-black/20 px-3 py-2"
          >
            <StatusDot tone="primary" pulse />
            <span className="text-sm font-medium">{guest.serverName}</span>
            <Badge variant="outline">
              {guest.localAddress}:{guest.localPort}
            </Badge>
            {guest.rttMs != null ? (
              <Badge variant="outline">{guest.rttMs} ms</Badge>
            ) : null}
            {guest.mode === "relay" ? <Badge variant="warning">relay</Badge> : null}
            <Button
              variant="ghost"
              size="sm"
              className="ml-auto hover:text-[var(--destructive)]"
              onClick={() => leave.mutate(guest.id)}
              loading={leave.isPending && leave.variables === guest.id}
            >
              <LogOut className="size-3.5" /> Leave
            </Button>
          </div>
        ))}
      </CardContent>
    </Card>
  );
}
