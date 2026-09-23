import { useEffect, useState, type CSSProperties } from "react";

import { useTranslation } from "react-i18next";

import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Stat } from "@/components/ui/feedback";
import { useRefreshAccount, useRefreshSkin } from "@/hooks/queries";
import {
  bodyUrl,
  bodyUrlFallback,
  capeUrl,
  elySkinUrl,
  fallbackGradient,
  monogram,
  publicTextureUrl,
  textureSlices,
  type Tile,
} from "@/lib/skins";
import { cn } from "@/lib/utils";
import { PROVIDER_LABEL, type AccountSummary, type SkinModel } from "@/types/account";

/** Front faces of the 64×64 layout. Slim arms are 3px wide. */
function frontTiles(model: SkinModel): Record<string, Tile> {
  const arm = model === "slim" ? 3 : 4;
  return {
    head: { x: 8, y: 8, w: 8, h: 8 },
    hat: { x: 40, y: 8, w: 8, h: 8 },
    body: { x: 20, y: 20, w: 8, h: 12 },
    bodyOverlay: { x: 20, y: 36, w: 8, h: 12 },
    rightArm: { x: 44, y: 20, w: arm, h: 12 },
    rightArmOverlay: { x: 44, y: 36, w: arm, h: 12 },
    leftArm: { x: 36, y: 52, w: arm, h: 12 },
    leftArmOverlay: { x: 52, y: 52, w: arm, h: 12 },
    rightLeg: { x: 4, y: 20, w: 4, h: 12 },
    rightLegOverlay: { x: 4, y: 36, w: 4, h: 12 },
    leftLeg: { x: 20, y: 52, w: 4, h: 12 },
    leftLegOverlay: { x: 4, y: 52, w: 4, h: 12 },
  };
}

function SkinPart({
  url,
  base,
  overlay,
  scale,
  className,
}: {
  url: string;
  base: Tile;
  overlay?: Tile;
  scale: number;
  className?: string;
}) {
  return (
    <span
      className={cn("relative block [image-rendering:pixelated]", className)}
      style={textureSlices(url, base, scale) as CSSProperties}
    >
      {overlay ? (
        <span
          className="absolute inset-0 [image-rendering:pixelated]"
          style={{
            ...(textureSlices(url, overlay, scale) as CSSProperties),
            width: "100%",
            height: "100%",
          }}
        />
      ) : null}
    </span>
  );
}

/** Front view sliced from the provider texture. mc-heads does not know Ely.by skins. */
export function SkinFigure({
  url,
  model,
  height = 224,
}: {
  url: string;
  model: SkinModel;
  height?: number;
}) {
  const tiles = frontTiles(model);
  const scale = height / 32;
  const arm = model === "slim" ? 3 : 4;
  const width = (arm * 2 + 8) * scale;
  return (
    <span className="relative mb-2 block" style={{ width, height }}>
      <span className="absolute" style={{ left: arm * scale, top: 0 }}>
        <SkinPart url={url} base={tiles.head!} overlay={tiles.hat} scale={scale} />
      </span>
      <span className="absolute" style={{ left: 0, top: 8 * scale }}>
        <SkinPart url={url} base={tiles.rightArm!} overlay={tiles.rightArmOverlay} scale={scale} />
      </span>
      <span className="absolute" style={{ left: arm * scale, top: 8 * scale }}>
        <SkinPart url={url} base={tiles.body!} overlay={tiles.bodyOverlay} scale={scale} />
      </span>
      <span className="absolute" style={{ left: (arm + 8) * scale, top: 8 * scale }}>
        <SkinPart url={url} base={tiles.leftArm!} overlay={tiles.leftArmOverlay} scale={scale} />
      </span>
      <span className="absolute" style={{ left: arm * scale, top: 20 * scale }}>
        <SkinPart url={url} base={tiles.rightLeg!} overlay={tiles.rightLegOverlay} scale={scale} />
      </span>
      <span className="absolute" style={{ left: (arm + 4) * scale, top: 20 * scale }}>
        <SkinPart url={url} base={tiles.leftLeg!} overlay={tiles.leftLegOverlay} scale={scale} />
      </span>
    </span>
  );
}

/**
 * Skin, cape and model preview for an account.
 *
 * Ely.by and Mojang textures are sliced from the PNG the account already
 * carries. mc-heads is only a fallback for Microsoft and offline profiles:
 * an Ely.by UUID is not a Mojang profile, so that service draws Steve.
 */
export function SkinPreview({ account, className }: { account: AccountSummary; className?: string }) {
  const { t } = useTranslation();
  const refresh = useRefreshSkin();
  const refreshAccount = useRefreshAccount();
  const cape = publicTextureUrl(capeUrl(account));
  const ownTexture = account.provider === "ely_by" || account.provider === "sx_acc";
  const texture =
    account.provider === "ely_by" ? elySkinUrl(account) : publicTextureUrl(account.skin.skinUrl);
  const [mode, setMode] = useState<"loading" | "texture" | "heads" | "fallback" | "mono">("loading");

  useEffect(() => {
    if (!texture) {
      setMode(ownTexture ? "mono" : "heads");
      return;
    }
    let cancelled = false;
    const img = new Image();
    img.onload = () => {
      if (!cancelled) setMode("texture");
    };
    img.onerror = () => {
      if (!cancelled) setMode(ownTexture ? "mono" : "heads");
    };
    img.src = texture;
    return () => {
      cancelled = true;
    };
  }, [texture, account.provider, account.id, ownTexture]);

  const bodySrc = mode === "heads" ? bodyUrl(account, 320) : mode === "fallback" ? bodyUrlFallback(account, 320) : null;
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
          {mode === "texture" && texture ? (
            <SkinFigure url={texture} model={account.skin.model} />
          ) : mode === "mono" || mode === "loading" ? (
            <div
              className="mb-6 flex size-32 items-center justify-center rounded-2xl text-4xl font-semibold text-white/90"
              style={{ background: fallbackGradient(account.username) }}
            >
              {mode === "loading" ? "" : monogram(account.username)}
            </div>
          ) : (
            <>
              <img
                src={bodySrc!}
                alt={`${account.username}'s skin`}
                className="mb-2 h-64 object-contain [image-rendering:pixelated] drop-shadow-[0_18px_35px_rgba(0,0,0,0.55)]"
                onError={() => setMode((current) => (current === "heads" ? "fallback" : "mono"))}
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
            ? "Offline profiles render the default skin. Sign in with Microsoft, Ely.by, or sx.acc to use your own custom skin."
            : account.provider === "ely_by"
              ? t("profile.authlibNote")
              : account.provider === "sx_acc"
                ? t("profile.authlibSxacc")
                : "Microsoft accounts use the official Mojang skin service — change your skin at minecraft.net and press “Reload skin” to fetch it here."}
        </p>
      </CardContent>
    </Card>
  );
}
