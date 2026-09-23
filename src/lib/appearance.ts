/**
 * Applies Nebula Vault appearance to the document.
 *
 * Theme presets, accent chips, density and motion come from `AppSettings`.
 * Font scale is frontend-only (no settings field yet) and lives in localStorage.
 */

import { useAccentMarkStore } from "@/stores/accent";
import type { AppSettings } from "@/types/system";

export const THEME_PRESETS = [
  { id: "nebula", label: "Nebula", hint: "Charcoal, galactic purple", colors: ["#101012", "#1C1C20", "#8B5CF6"] },
  { id: "obsidian", label: "Obsidian", hint: "Charcoal and zinc", colors: ["#08080A", "#1A1A1E", "#A1A1AA"] },
  { id: "aurora", label: "Aurora", hint: "Emerald and teal", colors: ["#041210", "#0F2A26", "#10B981"] },
  { id: "ember", label: "Ember", hint: "Warm amber", colors: ["#120808", "#261412", "#F97316"] },
  { id: "frost", label: "Frost", hint: "Sky cyan", colors: ["#060B14", "#121C2E", "#38BDF8"] },
  { id: "custom", label: "Custom", hint: "Charcoal base, your accent", colors: ["#101012", "#1C1C20", "#8B5CF6"] },
] as const;

/**
 * Locked swatch set. Each id is a crystalline S PNG in `public/brand/`.
 * Purple is the default galactic accent.
 */
export const ACCENT_PRESETS = [
  { id: "purple", label: "Purple", hex: "#8B5CF6" },
  { id: "cyan", label: "Cyan", hex: "#38BDF8" },
  { id: "magenta", label: "Magenta", hex: "#E879F9" },
  { id: "emerald", label: "Emerald", hex: "#10B981" },
  { id: "amber", label: "Amber", hex: "#F59E0B" },
  { id: "silver", label: "Silver", hex: "#D4D4D8" },
] as const;

export type AccentId = (typeof ACCENT_PRESETS)[number]["id"];

/** Settings chips are the swatch presets. */
export const ACCENT_CHIPS = ACCENT_PRESETS;

/** Older saved ids still resolve onto a swatch. */
const LEGACY_ACCENT: Record<string, AccentId> = {
  violet: "purple",
  purple: "purple",
  fuchsia: "magenta",
  magenta: "magenta",
  indigo: "purple",
  cyan: "cyan",
  emerald: "emerald",
  amber: "amber",
  silver: "silver",
};

const FONT_SCALE_KEY = "sxmlauncher.font-scale";

export type AppearanceSlice = Pick<
  AppSettings,
  "theme" | "uiAccent" | "uiAnimations" | "reduceMotion" | "uiCompact"
>;

export function normalizeTheme(theme: string | null | undefined): string {
  if (!theme || theme === "dark" || theme === "default" || theme === "system") return "nebula";
  return theme;
}

/** `null` when the player has not picked a scale, so density can own `--font-scale`. */
export function readFontScale(): number | null {
  try {
    const stored = localStorage.getItem(FONT_SCALE_KEY);
    if (stored == null) return null;
    const raw = Number(stored);
    if (!Number.isFinite(raw)) return null;
    return Math.min(1.1, Math.max(0.9, raw));
  } catch {
    return null;
  }
}

export function writeFontScale(scale: number): void {
  const next = Math.min(1.1, Math.max(0.9, scale));
  try {
    localStorage.setItem(FONT_SCALE_KEY, String(next));
  } catch {
    // Private mode: the CSS var still updates for this session.
  }
  document.documentElement.style.setProperty("--font-scale", next.toFixed(2));
}

const HEX_COLOR = /^#([0-9a-f]{6})$/i;

function hexAlpha(hex: string, alpha: number): string {
  const n = hex.replace("#", "");
  const r = Number.parseInt(n.slice(0, 2), 16);
  const g = Number.parseInt(n.slice(2, 4), 16);
  const b = Number.parseInt(n.slice(4, 6), 16);
  return `rgba(${r}, ${g}, ${b}, ${alpha})`;
}

/** Mix a hex toward white so secondary accent text stays readable. */
function lighten(hex: string, amount = 0.42): string {
  const n = hex.replace("#", "");
  const mix = (channel: number) => Math.round(channel + (255 - channel) * amount);
  const r = mix(Number.parseInt(n.slice(0, 2), 16));
  const g = mix(Number.parseInt(n.slice(2, 4), 16));
  const b = mix(Number.parseInt(n.slice(4, 6), 16));
  return `#${r.toString(16).padStart(2, "0")}${g.toString(16).padStart(2, "0")}${b.toString(16).padStart(2, "0")}`;
}

export interface ResolvedAccent {
  /** Preset id, or `custom` when the picker hex is not one of the six. */
  id: string;
  /** Color painted into `--accent`. */
  hex: string;
  /** PNG slot. Always one of the six swatches. */
  mark: AccentId;
}

function channel(hex: string, start: number): number {
  return Number.parseInt(hex.replace("#", "").slice(start, start + 2), 16);
}

/** Closest swatch when the player picks a hex that is not a preset. */
export function nearestAccent(hex: string): AccentId {
  const source = hex.toLowerCase();
  let best: AccentId = "purple";
  let bestDistance = Number.POSITIVE_INFINITY;
  for (const preset of ACCENT_PRESETS) {
    const target = preset.hex.toLowerCase();
    const distance =
      (channel(source, 0) - channel(target, 0)) ** 2 +
      (channel(source, 2) - channel(target, 2)) ** 2 +
      (channel(source, 4) - channel(target, 4)) ** 2;
    if (distance < bestDistance) {
      best = preset.id;
      bestDistance = distance;
    }
  }
  return best;
}

/** Preset id when the value is a known name; otherwise the nearest swatch. */
export function markForAccent(uiAccent: string | null | undefined): AccentId {
  if (uiAccent && LEGACY_ACCENT[uiAccent]) return LEGACY_ACCENT[uiAccent];
  if (uiAccent && HEX_COLOR.test(uiAccent)) {
    const exact = ACCENT_PRESETS.find((preset) => preset.hex.toLowerCase() === uiAccent.toLowerCase());
    return exact?.id ?? nearestAccent(uiAccent);
  }
  return "purple";
}

/** Sidebar uses 32, the header wordmark uses 64. */
export function markSrc(id: AccentId, slot: 32 | 64): string {
  return `/brand/sxmlauncher-mark-${id}-${slot}.png`;
}

/**
 * Named presets or a live `#rrggbb` from the color picker.
 * Unknown names fall back to galactic purple.
 */
export function resolveAccent(uiAccent: string | null | undefined): ResolvedAccent {
  const mark = markForAccent(uiAccent);
  if (uiAccent && HEX_COLOR.test(uiAccent)) {
    const exact = ACCENT_PRESETS.find((preset) => preset.hex.toLowerCase() === uiAccent.toLowerCase());
    if (exact) return { id: exact.id, hex: exact.hex.toLowerCase(), mark: exact.id };
    return { id: "custom", hex: uiAccent.toLowerCase(), mark };
  }
  const preset = ACCENT_PRESETS.find((entry) => entry.id === mark) ?? ACCENT_PRESETS[0];
  return { id: preset.id, hex: preset.hex.toLowerCase(), mark: preset.id };
}

/** Paint theme, accent, density and motion onto `:root`. Safe to call often. */
export function applyAppearance(settings: AppearanceSlice): void {
  const root = document.documentElement;
  const accent = resolveAccent(settings.uiAccent);

  root.dataset.theme = normalizeTheme(settings.theme);
  root.dataset.density = settings.uiCompact ? "compact" : "comfortable";
  root.dataset.accent = accent.hex;
  root.dataset.accentId = accent.mark;
  useAccentMarkStore.getState().setMark(accent.mark);
  root.classList.add("dark");
  root.classList.toggle("no-animations", !settings.uiAnimations || settings.reduceMotion);

  root.style.setProperty("--accent", accent.hex);
  root.style.setProperty("--accent-soft", lighten(accent.hex));
  root.style.setProperty("--accent-dim", hexAlpha(accent.hex, 0.22));
  root.style.setProperty("--accent-glow", hexAlpha(accent.hex, 0.38));
  root.style.setProperty("--rim-light", hexAlpha(accent.hex, 0.55));
  root.style.setProperty("--primary", accent.hex);
  root.style.setProperty("--ring", hexAlpha(accent.hex, 0.6));
  const scale = readFontScale();
  if (scale == null) root.style.removeProperty("--font-scale");
  else root.style.setProperty("--font-scale", scale.toFixed(2));
}
