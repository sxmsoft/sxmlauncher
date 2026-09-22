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
    <header className={cn("flex flex-wrap items-end justify-between gap-4", className)}>
      <div className="flex flex-col gap-1">
        <h1 className="text-[22px] leading-none font-semibold tracking-[-0.02em]">{title}</h1>
        {description ? (
          <p className="max-w-2xl text-[13px] text-[var(--text-muted)]">{description}</p>
        ) : null}
      </div>
      {actions ? <div className="flex flex-wrap items-center gap-2">{actions}</div> : null}
    </header>
  );
}
