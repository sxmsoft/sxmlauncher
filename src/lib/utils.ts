import { clsx, type ClassValue } from "clsx";
import { twMerge } from "tailwind-merge";

/** Merge conditional class names with Tailwind conflict resolution. */
export function cn(...inputs: ClassValue[]) {
  return twMerge(clsx(inputs));
}

/** Human-readable byte size (1024-based). */
export function formatBytes(bytes: number, fractionDigits = 1): string {
  if (!Number.isFinite(bytes) || bytes <= 0) return "0 B";
  const units = ["B", "KiB", "MiB", "GiB", "TiB"];
  const exponent = Math.min(
    Math.floor(Math.log(bytes) / Math.log(1024)),
    units.length - 1,
  );
  const value = bytes / 1024 ** exponent;
  return `${value.toFixed(exponent === 0 ? 0 : fractionDigits)} ${units[exponent]}`;
}

/** `1h 12m`, `4m 05s`, or `12s`. */
export function formatDuration(totalSeconds: number): string {
  const seconds = Math.max(0, Math.floor(totalSeconds));
  const hours = Math.floor(seconds / 3600);
  const minutes = Math.floor((seconds % 3600) / 60);
  const rest = seconds % 60;
  if (hours > 0) return `${hours}h ${String(minutes).padStart(2, "0")}m`;
  if (minutes > 0) return `${minutes}m ${String(rest).padStart(2, "0")}s`;
  return `${rest}s`;
}

/** Relative time such as `just now`, `3 min ago`, `2 days ago`. */
export function formatRelative(iso: string | number | Date): string {
  const then = new Date(iso).getTime();
  if (Number.isNaN(then)) return "unknown";
  const seconds = Math.round((Date.now() - then) / 1000);
  if (seconds < 45) return "just now";
  if (seconds < 90) return "1 min ago";
  const minutes = Math.round(seconds / 60);
  if (minutes < 60) return `${minutes} min ago`;
  const hours = Math.round(minutes / 60);
  if (hours < 24) return `${hours}h ago`;
  const days = Math.round(hours / 24);
  return days === 1 ? "yesterday" : `${days} days ago`;
}

/** Percentage `0..=100` for a completed/total pair. */
export function percent(completed: number, total: number): number {
  if (total <= 0) return 0;
  return Math.min(100, Math.max(0, (completed / total) * 100));
}

/** Colour for a latency badge. */
export function pingTone(pingMs: number | null | undefined): string {
  if (pingMs == null) return "text-muted-foreground";
  if (pingMs < 60) return "text-[var(--success)]";
  if (pingMs < 150) return "text-[var(--warning)]";
  return "text-[var(--destructive)]";
}

/** Stable colour per loader, used for version badges. */
export function loaderTone(loader: string): string {
  switch (loader) {
    case "fabric":
      return "bg-amber-500/15 text-amber-300 border-amber-400/25";
    case "quilt":
      return "bg-fuchsia-500/15 text-fuchsia-300 border-fuchsia-400/25";
    case "forge":
      return "bg-orange-500/15 text-orange-300 border-orange-400/25";
    case "neoforge":
      return "bg-rose-500/15 text-rose-300 border-rose-400/25";
    default:
      return "bg-slate-500/15 text-slate-300 border-slate-400/25";
  }
}

/** Copy text to the clipboard, returning whether it worked. */
export async function copyToClipboard(text: string): Promise<boolean> {
  try {
    await navigator.clipboard.writeText(text);
    return true;
  } catch {
    return false;
  }
}
