import * as React from "react";

import { cn } from "@/lib/utils";

/** Frosted panel — the base surface for every page section. */
export function Card({ className, ...props }: React.ComponentProps<"div">) {
  return (
    <div
      className={cn(
        "glass rounded-[var(--radius-xl)] text-[var(--card-foreground)] shadow-[0_20px_60px_-30px_rgba(0,0,0,0.9)]",
        className,
      )}
      {...props}
    />
  );
}

export function CardHeader({ className, ...props }: React.ComponentProps<"div">) {
  return <div className={cn("flex flex-col gap-1 p-5 pb-3", className)} {...props} />;
}

export function CardTitle({ className, ...props }: React.ComponentProps<"h3">) {
  return (
    <h3
      className={cn("text-base leading-tight font-semibold tracking-tight", className)}
      {...props}
    />
  );
}

export function CardDescription({ className, ...props }: React.ComponentProps<"p">) {
  return <p className={cn("text-muted-foreground text-sm", className)} {...props} />;
}

export function CardContent({ className, ...props }: React.ComponentProps<"div">) {
  return <div className={cn("p-5 pt-0", className)} {...props} />;
}

export function CardFooter({ className, ...props }: React.ComponentProps<"div">) {
  return <div className={cn("flex items-center gap-2 p-5 pt-0", className)} {...props} />;
}
