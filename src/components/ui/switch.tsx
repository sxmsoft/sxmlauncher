import * as React from "react";

import { cn } from "@/lib/utils";

/**
 * Accessible toggle.
 *
 * Deliberately not a Radix primitive: the launcher's settings panels are full of
 * these and a plain `role="switch"` button keeps the bundle smaller while
 * behaving identically for keyboard and screen-reader users.
 */
export function Switch({
  checked,
  onCheckedChange,
  disabled,
  className,
  ...props
}: Omit<React.ComponentProps<"button">, "onChange" | "children"> & {
  checked: boolean;
  onCheckedChange: (checked: boolean) => void;
}) {
  return (
    <button
      type="button"
      role="switch"
      aria-checked={checked}
      disabled={disabled}
      onClick={() => onCheckedChange(!checked)}
      className={cn(
        "relative inline-flex h-6 w-11 shrink-0 cursor-pointer items-center rounded-full border transition-colors duration-200 outline-none focus-visible:ring-2 focus-visible:ring-[var(--ring)] disabled:cursor-not-allowed disabled:opacity-50",
        checked
          ? "border-transparent bg-[var(--primary)]"
          : "border-white/12 bg-white/10",
        className,
      )}
      {...props}
    >
      <span
        className={cn(
          "pointer-events-none block size-4.5 rounded-full bg-white shadow transition-transform duration-200",
          checked ? "translate-x-5.5" : "translate-x-0.5",
        )}
      />
    </button>
  );
}

/** Number input with an inline unit suffix (memory, resolution, ports). */
export function NumberInput({
  value,
  onValueChange,
  suffix,
  className,
  min,
  max,
  step = 1,
  ...props
}: Omit<React.ComponentProps<"input">, "onChange" | "value" | "type"> & {
  value: number;
  onValueChange: (value: number) => void;
  suffix?: string;
  min?: number;
  max?: number;
}) {
  return (
    <div
      className={cn(
        "flex h-10 items-center gap-2 rounded-lg border border-[var(--input)] bg-black/25 px-3 focus-within:border-[color-mix(in_oklab,var(--primary)_60%,transparent)] focus-within:ring-2 focus-within:ring-[var(--ring)]",
        className,
      )}
    >
      <input
        type="number"
        className="w-full bg-transparent text-sm outline-none [appearance:textfield] [&::-webkit-inner-spin-button]:appearance-none"
        value={Number.isFinite(value) ? value : ""}
        min={min}
        max={max}
        step={step}
        onChange={(event) => {
          const next = Number(event.target.value);
          if (!Number.isNaN(next)) onValueChange(next);
        }}
        {...props}
      />
      {suffix ? <span className="text-muted-foreground text-xs">{suffix}</span> : null}
    </div>
  );
}
