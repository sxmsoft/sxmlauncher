import * as React from "react";

import * as ProgressPrimitive from "@radix-ui/react-progress";

import { cn } from "@/lib/utils";

export function Progress({
  className,
  value = 0,
  tone = "primary",
  ...props
}: React.ComponentProps<typeof ProgressPrimitive.Root> & {
  tone?: "primary" | "success" | "warning" | "destructive";
}) {
  const clamped = Math.min(100, Math.max(0, value ?? 0));
  const fill = {
    primary: "bg-[var(--primary)]",
    success: "bg-[var(--success)]",
    warning: "bg-[var(--warning)]",
    destructive: "bg-[var(--destructive)]",
  }[tone];

  return (
    <ProgressPrimitive.Root
      className={cn("relative h-2 w-full overflow-hidden rounded-full bg-white/10", className)}
      value={clamped}
      {...props}
    >
      <ProgressPrimitive.Indicator
        className={cn("h-full w-full rounded-full transition-transform duration-300 ease-out", fill)}
        style={{ transform: `translateX(-${100 - clamped}%)` }}
      />
    </ProgressPrimitive.Root>
  );
}

/**
 * Progress bar for a job with no known unit count.
 *
 * The backend sends `totalUnits: 0` while it is still resolving, which would
 * otherwise render as a stuck empty bar; an indeterminate shimmer reads as
 * "working" instead.
 */
export function IndeterminateProgress({ className }: { className?: string }) {
  return (
    <div className={cn("relative h-2 w-full overflow-hidden rounded-full bg-white/10", className)}>
      <div className="shimmer h-full w-full bg-[var(--primary)]/40" />
    </div>
  );
}
