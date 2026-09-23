/**
 * Crystalline S mark. The file is chosen from the accent preset id
 * (`purple`, `cyan`, `magenta`, `emerald`, `amber`, `silver`).
 * Sidebar uses the 32px PNG; the header wordmark uses the 64px PNG.
 */

import { markSrc, type AccentId } from "@/lib/appearance";
import { cn } from "@/lib/utils";
import { useAccentMarkStore } from "@/stores/accent";

export function BrandMark({
  slot = 32,
  className,
}: {
  /** Asset slot. Sidebar is 32, header is 64. */
  slot?: 32 | 64;
  className?: string;
}) {
  const mark = useAccentMarkStore((state) => state.mark);
  return (
    <img
      src={markSrc(mark, slot)}
      alt=""
      width={slot}
      height={slot}
      draggable={false}
      data-brand-slot="mark"
      data-accent-id={mark satisfies AccentId}
      className={cn("pointer-events-none object-contain", className)}
    />
  );
}
