import * as React from "react";

import { Slot } from "@radix-ui/react-slot";
import { cva, type VariantProps } from "class-variance-authority";
import { LoaderCircle } from "lucide-react";

import { cn } from "@/lib/utils";

const buttonVariants = cva(
  "inline-flex shrink-0 items-center justify-center gap-2 whitespace-nowrap rounded-[var(--radius)] text-sm font-semibold tracking-tight outline-none transition-colors duration-150 focus-visible:ring-2 focus-visible:ring-[var(--ring)] focus-visible:ring-offset-2 focus-visible:ring-offset-[var(--background)] disabled:pointer-events-none disabled:opacity-50 [&_svg]:size-4 [&_svg]:shrink-0",
  {
    variants: {
      variant: {
        default:
          "bg-[var(--primary)] text-[var(--primary-foreground)] hover:brightness-105",
        secondary:
          "bg-[var(--secondary)] text-[var(--secondary-foreground)] hover:bg-[color-mix(in_oklab,var(--secondary)_80%,white)]",
        outline:
          "border border-[var(--glass-border)] bg-white/5 text-[var(--foreground)] hover:bg-white/10",
        ghost: "text-[var(--muted-foreground)] hover:bg-white/8 hover:text-[var(--foreground)]",
        destructive:
          "bg-[var(--destructive)] text-[var(--destructive-foreground)] hover:brightness-110",
        success: "bg-[var(--success)] text-[oklch(0.16_0.02_150)] hover:brightness-105",
        link: "text-[var(--primary)] underline-offset-4 hover:underline",
      },
      size: {
        sm: "h-8 px-3 text-xs",
        default: "h-10 px-4",
        lg: "h-12 px-6 text-base",
        xl: "h-14 px-8 text-base font-semibold",
        icon: "size-9",
        "icon-sm": "size-7 rounded-md",
      },
    },
    defaultVariants: { variant: "default", size: "default" },
  },
);

export interface ButtonProps
  extends React.ComponentProps<"button">,
    VariantProps<typeof buttonVariants> {
  /** Render as the single child element instead of a `<button>`. */
  asChild?: boolean;
  /** Show a spinner and disable the button. */
  loading?: boolean;
}

export function Button({
  className,
  variant,
  size,
  asChild = false,
  loading = false,
  disabled,
  children,
  ...props
}: ButtonProps) {
  const Component = asChild ? Slot : "button";
  return (
    <Component
      className={cn(buttonVariants({ variant, size }), className)}
      disabled={disabled || loading}
      {...props}
    >
      {loading ? (
        <>
          <LoaderCircle className="animate-spin" aria-hidden />
          {children}
        </>
      ) : (
        children
      )}
    </Component>
  );
}

export { buttonVariants };
