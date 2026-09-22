import { useEffect } from "react";

import { AnimatePresence, motion } from "framer-motion";
import { CircleAlert, CircleCheck, CircleX, Info, X } from "lucide-react";

import { cn } from "@/lib/utils";
import { useUiStore, type Toast } from "@/stores/ui";
import type { ToastTone } from "@/types/system";

const LIFETIME_MS = 6_000;

const toneStyles: Record<ToastTone, { icon: typeof Info; className: string }> = {
  info: { icon: Info, className: "text-[var(--foreground)]" },
  success: { icon: CircleCheck, className: "text-[var(--success)]" },
  warning: { icon: CircleAlert, className: "text-[var(--warning)]" },
  error: { icon: CircleX, className: "text-[var(--destructive)]" },
};

function ToastRow({ toast }: { toast: Toast }) {
  const dismiss = useUiStore((state) => state.dismissToast);
  const { icon: Icon, className } = toneStyles[toast.tone];

  useEffect(() => {
    const timer = window.setTimeout(() => dismiss(toast.id), LIFETIME_MS);
    return () => window.clearTimeout(timer);
  }, [dismiss, toast.id]);

  return (
    <motion.div
      layout
      initial={{ opacity: 0, y: 12, scale: 0.97 }}
      animate={{ opacity: 1, y: 0, scale: 1 }}
      exit={{ opacity: 0, y: 8, scale: 0.97 }}
      transition={{ duration: 0.18, ease: "easeOut" }}
      className="glass-strong pointer-events-auto flex w-80 items-start gap-3 rounded-xl p-3 shadow-2xl"
      role="status"
    >
      <Icon className={cn("mt-0.5 size-4 shrink-0", className)} />
      <div className="flex min-w-0 flex-1 flex-col gap-0.5">
        <span className="text-sm leading-snug font-medium">{toast.title}</span>
        {toast.message ? (
          <span className="text-muted-foreground text-xs leading-relaxed break-words">
            {toast.message}
          </span>
        ) : null}
      </div>
      <button
        type="button"
        onClick={() => dismiss(toast.id)}
        className="text-muted-foreground rounded p-0.5 transition-colors hover:bg-white/10 hover:text-[var(--foreground)]"
        aria-label="Dismiss"
      >
        <X className="size-3.5" />
      </button>
    </motion.div>
  );
}

/** Toast viewport — mounted once, bottom-right, above the status strip. */
export function Toaster() {
  const toasts = useUiStore((state) => state.toasts);
  return (
    <div className="pointer-events-none fixed right-4 bottom-10 z-[100] flex flex-col gap-2">
      <AnimatePresence initial={false}>
        {toasts.map((toast) => (
          <ToastRow key={toast.id} toast={toast} />
        ))}
      </AnimatePresence>
    </div>
  );
}
