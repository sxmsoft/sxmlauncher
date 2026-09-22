import * as React from "react";

import { cva, type VariantProps } from "class-variance-authority";

import { cn } from "@/lib/utils";

const badgeVariants = cva(
  "inline-flex items-center gap-1 rounded-full border px-2 py-0.5 text-[11px] font-medium whitespace-nowrap [&_svg]:size-3 [&_svg]:shrink-0",
  {
    variants: {
      variant: {
        default: "border-white/12 bg-white/8 text-[var(--foreground)]",
        primary: "border-[color-mix(in_oklab,var(--primary)_45%,transparent)] bg-[color-mix(in_oklab,var(--primary)_18%,transparent)] text-[var(--foreground)]",
        outline: "border-white/15 bg-transparent text-[var(--muted-foreground)]",
        success: "border-[color-mix(in_oklab,var(--success)_45%,transparent)] bg-[color-mix(in_oklab,var(--success)_16%,transparent)] text-[var(--success)]",
        warning: "border-[color-mix(in_oklab,var(--warning)_45%,transparent)] bg-[color-mix(in_oklab,var(--warning)_16%,transparent)] text-[var(--warning)]",
        destructive: "border-[color-mix(in_oklab,var(--destructive)_45%,transparent)] bg-[color-mix(in_oklab,var(--destructive)_16%,transparent)] text-[var(--destructive)]",
      },
    },
    defaultVariants: { variant: "default" },
  },
);

export interface BadgeProps
  extends React.ComponentProps<"span">,
    VariantProps<typeof badgeVariants> {}

export function Badge({ className, variant, ...props }: BadgeProps) {
  return <span className={cn(badgeVariants({ variant }), className)} {...props} />;
}

/** A small status dot, used on live server cards and running instances. */
export function StatusDot({
  tone = "muted",
  pulse = false,
  className,
}: {
  tone?: "success" | "warning" | "destructive" | "primary" | "muted";
  pulse?: boolean;
  className?: string;
}) {
  const colour = {
    success: "bg-[var(--success)]",
    warning: "bg-[var(--warning)]",
    destructive: "bg-[var(--destructive)]",
    primary: "bg-[var(--primary)]",
    muted: "bg-[var(--muted-foreground)]",
  }[tone];

  return (
    <span className={cn("relative flex size-2", className)}>
      {pulse && (
        <span className={cn("absolute inset-0 animate-ping rounded-full opacity-70", colour)} />
      )}
      <span className={cn("relative inline-flex size-2 rounded-full", colour)} />
    </span>
  );
}
