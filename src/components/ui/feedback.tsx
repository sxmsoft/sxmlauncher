import * as React from "react";

import { LoaderCircle, TriangleAlert } from "lucide-react";

import { cn } from "@/lib/utils";
import { Button } from "./button";
import { Card } from "./card";

export function Skeleton({ className, ...props }: React.ComponentProps<"div">) {
  return (
    <div className={cn("animate-pulse rounded-lg bg-white/8", className)} {...props} />
  );
}

export function Separator({
  className,
  orientation = "horizontal",
}: {
  className?: string;
  orientation?: "horizontal" | "vertical";
}) {
  return (
    <div
      role="separator"
      className={cn(
        "bg-white/8",
        orientation === "horizontal" ? "h-px w-full" : "h-full w-px",
        className,
      )}
    />
  );
}

export function Spinner({ className }: { className?: string }) {
  return <LoaderCircle className={cn("size-4 animate-spin", className)} aria-hidden />;
}

/** Small key/value pair used across instance and server cards. */
export function Stat({
  label,
  value,
  tone,
  className,
}: {
  label: string;
  value: React.ReactNode;
  tone?: string;
  className?: string;
}) {
  return (
    <div className={cn("flex flex-col gap-0.5", className)}>
      <span className="text-muted-foreground text-[10px] font-medium tracking-widest uppercase">
        {label}
      </span>
      <span className={cn("text-sm font-medium tabular-nums", tone)}>{value}</span>
    </div>
  );
}

export function EmptyState({
  icon,
  title,
  description,
  action,
  className,
}: {
  icon?: React.ReactNode;
  title: string;
  description?: string;
  action?: React.ReactNode;
  className?: string;
}) {
  return (
    <div
      className={cn(
        "flex flex-col items-center justify-center gap-3 rounded-[var(--radius-xl)] border border-dashed border-white/12 px-6 py-12 text-center",
        className,
      )}
    >
      {icon ? <div className="text-muted-foreground [&_svg]:size-6">{icon}</div> : null}
      <div className="flex flex-col gap-1">
        <p className="text-sm font-medium">{title}</p>
        {description ? (
          <p className="text-muted-foreground max-w-md text-xs leading-relaxed">{description}</p>
        ) : null}
      </div>
      {action}
    </div>
  );
}

export function ErrorState({
  title = "Something went wrong",
  message,
  onRetry,
  className,
}: {
  title?: string;
  message: string;
  onRetry?: () => void;
  className?: string;
}) {
  return (
    <Card className={cn("flex flex-col items-start gap-3 p-5", className)}>
      <div className="flex items-center gap-2 text-[var(--destructive)]">
        <TriangleAlert className="size-4" />
        <span className="text-sm font-medium">{title}</span>
      </div>
      <p className="text-muted-foreground text-xs leading-relaxed">{message}</p>
      {onRetry ? (
        <Button size="sm" variant="outline" onClick={onRetry}>
          Try again
        </Button>
      ) : null}
    </Card>
  );
}

/** Loading placeholder shaped like a card, for list skeletons. */
export function CardSkeleton({ className }: { className?: string }) {
  return (
    <Card className={cn("flex flex-col gap-3 p-5", className)}>
      <Skeleton className="h-4 w-1/3" />
      <Skeleton className="h-3 w-2/3" />
      <Skeleton className="mt-2 h-9 w-full" />
    </Card>
  );
}
