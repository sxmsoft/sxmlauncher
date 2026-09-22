import type { ReactNode } from "react";

import { cn } from "@/lib/utils";

/** Title block shared by every page: heading, subtitle, and right-side actions. */
export function PageHeader({
  title,
  description,
  actions,
  className,
}: {
  title: string;
  description?: string;
  actions?: ReactNode;
  className?: string;
}) {
  return (
    <header className={cn("flex flex-wrap items-start justify-between gap-4", className)}>
      <div className="flex flex-col gap-1">
        <p className="text-[10px] font-semibold tracking-[0.22em] text-[var(--primary)] uppercase">
          SXM Deck
        </p>
        <h1 className="font-display text-4xl leading-none font-semibold">{title}</h1>
        {description ? (
          <p className="text-muted-foreground max-w-2xl text-sm">{description}</p>
        ) : null}
      </div>
      {actions ? <div className="flex flex-wrap items-center gap-2">{actions}</div> : null}
    </header>
  );
}
