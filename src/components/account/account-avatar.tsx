import { useEffect, useState } from "react";

import { cn } from "@/lib/utils";
import {
  fallbackGradient,
  headStyle,
  headUrl,
  headUrlFallback,
  monogram,
  skinTextureUrl,
} from "@/lib/skins";
import type { AccountSummary } from "@/types/account";

/**
 * Player head, with layered fallbacks.
 *
 * Order: raw provider texture (sliced, no third party) → mc-heads render →
 * crafatar render → deterministic gradient monogram. An offline profile still
 * gets a default Steve from the render services; only a network failure falls
 * back to the monogram.
 */
export function AccountAvatar({
  account,
  size = 32,
  className,
}: {
  account: AccountSummary;
  size?: number;
  className?: string;
}) {
  const [level, setLevel] = useState(0);
  const texture = skinTextureUrl(account);
  const slice = level === 0 ? headStyle(texture, size) : null;

  // CSS backgrounds never fire onError — probe the texture with an Image so a
  // dead URL (HTTP blocked, 204, etc.) advances to mc-heads / monogram.
  useEffect(() => {
    setLevel(0);
    if (!texture) {
      setLevel(1);
      return;
    }
    const img = new Image();
    img.onload = () => setLevel(0);
    img.onerror = () => setLevel(1);
    img.src = texture;
  }, [texture]);

  const renderSrc =
    level === 1 ? headUrl(account, size * 2) : level === 2 ? headUrlFallback(account, size * 2) : null;

  return (
    <span
      className={cn(
        "relative inline-flex shrink-0 items-center justify-center overflow-hidden rounded-md border border-white/12",
        className,
      )}
      style={{ width: size, height: size }}
    >
      {slice ? (
        <span className="relative block size-full [image-rendering:pixelated]" style={slice.face}>
          <span className="absolute inset-0 [image-rendering:pixelated]" style={slice.hat} />
        </span>
      ) : renderSrc ? (
        <img
          src={renderSrc}
          alt=""
          width={size}
          height={size}
          className="size-full object-cover [image-rendering:pixelated]"
          onError={() => setLevel((current) => current + 1)}
        />
      ) : (
        <span
          className="flex size-full items-center justify-center text-[0.7em] font-semibold text-white/90"
          style={{ background: fallbackGradient(account.username), fontSize: size / 2.4 }}
        >
          {monogram(account.username)}
        </span>
      )}
    </span>
  );
}
