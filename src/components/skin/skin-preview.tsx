import { useState } from "react";

import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Stat } from "@/components/ui/feedback";
import { useRefreshAccount, useRefreshSkin } from "@/hooks/queries";
import { bodyUrl, bodyUrlFallback, capeUrl, fallbackGradient, monogram, skinTextureUrl } from "@/lib/skins";
import { cn } from "@/lib/utils";
import { PROVIDER_LABEL, type AccountSummary } from "@/types/account";

/**
 * Skin, cape and model preview for an account.
 *
 * Ely.by textures come from the provider URL (mc-heads cannot resolve Ely UUIDs).
 * Microsoft / offline use mc-heads → crafatar → monogram.
 */
export function SkinPreview({ account, className }: { account: AccountSummary; className?: string }) {
  const [level, setLevel] = useState(0);
  const refresh = useRefreshSkin();
  const refreshAccount = useRefreshAccount();
  const cape = capeUrl(account);
  const texture = skinTextureUrl(account);

  const bodySrc =
    account.provider === "ely_by" && texture
      ? texture
      : level === 0
        ? bodyUrl(account, 320)
        : level === 1
          ? bodyUrlFallback(account, 320)
          : null;

  const showMonogram = bodySrc == null;
  const canRefresh = account.provider !== "offline";

  return (
    <Card className={cn("overflow-hidden", className)}>
      <CardHeader className="flex-row items-center justify-between">
        <CardTitle>{account.username}</CardTitle>
        <div className="flex items-center gap-2">
          <Badge variant={account.provider === "offline" ? "outline" : "primary"}>
            {PROVIDER_LABEL[account.provider]}
          </Badge>
          <Badge variant="outline">{account.skin.model === "slim" ? "slim (Alex)" : "classic (Steve)"}</Badge>
        </div>
      </CardHeader>

      <CardContent className="flex flex-col gap-4">
        <div className="relative flex h-72 items-end justify-center rounded-xl border border-white/8 bg-[radial-gradient(120%_90%_at_50%_0%,rgba(255,255,255,0.08),transparent)]">
          {showMonogram ? (
            <div
              className="mb-6 flex size-32 items-center justify-center rounded-2xl text-4xl font-semibold text-white/90"
              style={{ background: fallbackGradient(account.username) }}
            >
              {monogram(account.username)}
            </div>
          ) : (
            <>
              <img
                src={bodySrc!}
                alt={`${account.username}'s skin`}
                className={cn(
                  "mb-2 object-contain [image-rendering:pixelated] drop-shadow-[0_18px_35px_rgba(0,0,0,0.55)]",
                  account.provider === "ely_by" && texture ? "h-48 w-48" : "h-64",
                )}
                onError={() => {
                  if (account.provider === "ely_by") setLevel(2);
                  else setLevel((current) => current + 1);
                }}
              />
              {cape ? (
                <img
                  src={cape}
                  alt=""
                  className="absolute bottom-2 left-6 h-32 object-contain opacity-90 [image-rendering:pixelated]"
                  onError={(event) => {
                    event.currentTarget.style.display = "none";
                  }}
                />
              ) : null}
            </>
          )}
          {canRefresh ? (
            <Button
              variant="ghost"
              size="sm"
              className="absolute top-2 right-2"
              onClick={() => {
                refresh.mutate(account.id);
                if (account.hasStoredCredentials) refreshAccount.mutate(account.id);
              }}
              loading={refresh.isPending || refreshAccount.isPending}
            >
              Reload skin
            </Button>
          ) : null}
        </div>

        <div className="grid grid-cols-2 gap-4 sm:grid-cols-4">
          <Stat label="UUID" value={<span className="text-[11px] break-all">{account.uuid}</span>} />
          <Stat label="Cape" value={cape ? "equipped" : "none"} />
          <Stat
            label="Credentials"
            value={account.hasStoredCredentials ? "in vault" : "none"}
          />
          <Stat
            label="Session"
            value={
              account.expiresAt
                ? new Date(account.expiresAt).toLocaleTimeString()
                : "no expiry"
            }
          />
        </div>

        <p className="text-muted-foreground text-xs leading-relaxed">
          {account.provider === "offline"
            ? "Offline profiles render the default skin. Sign in with Microsoft or Ely.by to use your own custom skin."
            : account.provider === "ely_by"
              ? "Ely.by skins are attached to the JVM through the Authlib endpoint, so other Ely.by players see them in game. Changed your skin? Press “Reload skin”."
              : "Microsoft accounts use the official Mojang skin service — change your skin at minecraft.net and press “Reload skin” to fetch it here."}
        </p>
      </CardContent>
    </Card>
  );
}
