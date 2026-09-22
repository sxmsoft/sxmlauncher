import * as React from "react";

import { cn } from "@/lib/utils";

export function Input({ className, ...props }: React.ComponentProps<"input">) {
  return (
    <input
      className={cn(
        "h-10 w-full rounded-lg border border-[var(--input)] bg-black/25 px-3 text-sm text-[var(--foreground)] outline-none transition-colors placeholder:text-[var(--muted-foreground)] focus:border-[color-mix(in_oklab,var(--primary)_60%,transparent)] focus:ring-2 focus:ring-[var(--ring)] disabled:opacity-50",
        className,
      )}
      {...props}
    />
  );
}

export function Textarea({ className, ...props }: React.ComponentProps<"textarea">) {
  return (
    <textarea
      className={cn(
        "min-h-20 w-full resize-none rounded-lg border border-[var(--input)] bg-black/25 px-3 py-2 text-sm outline-none transition-colors placeholder:text-[var(--muted-foreground)] focus:border-[color-mix(in_oklab,var(--primary)_60%,transparent)] focus:ring-2 focus:ring-[var(--ring)]",
        className,
      )}
      {...props}
    />
  );
}

export function Label({ className, ...props }: React.ComponentProps<"label">) {
  return (
    <label
      className={cn("text-xs font-medium tracking-wide text-[var(--muted-foreground)] uppercase", className)}
      {...props}
    />
  );
}

/** Native select styled to match `Input` (no extra dependency needed). */
export function Select({ className, ...props }: React.ComponentProps<"select">) {
  return (
    <select
      className={cn(
        "h-10 w-full appearance-none rounded-lg border border-[var(--input)] bg-black/25 bg-[url('data:image/svg+xml;utf8,<svg xmlns=%22http://www.w3.org/2000/svg%22 viewBox=%220 0 24 24%22 fill=%22none%22 stroke=%22%2394a3b8%22 stroke-width=%222%22><path d=%22M6 9l6 6 6-6%22/></svg>')] bg-[length:16px] bg-[right_0.6rem_center] bg-no-repeat pr-9 pl-3 text-sm outline-none focus:border-[color-mix(in_oklab,var(--primary)_60%,transparent)] focus:ring-2 focus:ring-[var(--ring)]",
        className,
      )}
      {...props}
    />
  );
}

/** Label + control + hint, the standard row in the Settings page. */
export function Field({
  label,
  hint,
  htmlFor,
  className,
  children,
}: {
  label: string;
  hint?: string;
  htmlFor?: string;
  className?: string;
  children: React.ReactNode;
}) {
  return (
    <div className={cn("flex flex-col gap-1.5", className)}>
      <Label htmlFor={htmlFor}>{label}</Label>
      {children}
      {hint ? <p className="text-muted-foreground text-xs leading-snug">{hint}</p> : null}
    </div>
  );
}

/** A labelled row with the control on the right (toggles, switches). */
export function SettingRow({
  title,
  description,
  control,
  className,
}: {
  title: string;
  description?: string;
  control: React.ReactNode;
  className?: string;
}) {
  return (
    <div
      className={cn(
        "flex items-start justify-between gap-6 border-b border-white/6 py-3 last:border-b-0",
        className,
      )}
    >
      <div className="flex flex-col gap-0.5">
        <span className="text-sm font-medium">{title}</span>
        {description ? (
          <span className="text-muted-foreground text-xs leading-snug">{description}</span>
        ) : null}
      </div>
      <div className="shrink-0 pt-0.5">{control}</div>
    </div>
  );
}
