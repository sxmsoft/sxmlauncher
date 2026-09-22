import { useState } from "react";

import { Image as ImageIcon, Video, Zap } from "lucide-react";

import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Field, SettingRow } from "@/components/ui/input";
import { Switch } from "@/components/ui/switch";
import {
  ACCENT_CHIPS,
  normalizeTheme,
  readFontScale,
  THEME_PRESETS,
  writeFontScale,
} from "@/lib/appearance";
import { cn } from "@/lib/utils";
import type { AppSettings } from "@/types/system";

const BG_MODES: Array<{ id: "aurora" | "image" | "video"; label: string; description: string }> = [
  { id: "aurora", label: "Aurora", description: "Soft ambient glow" },
  { id: "image", label: "Photo", description: "PNG, JPG, WebP" },
  { id: "video", label: "Video", description: "MP4, WebM, MOV" },
];

async function pickBackgroundFile(kind: "image" | "video"): Promise<string | null> {
  const { open } = await import("@tauri-apps/plugin-dialog");
  const filters =
    kind === "image"
      ? [{ name: "Images", extensions: ["png", "jpg", "jpeg", "webp", "gif", "avif", "bmp"] }]
      : [{ name: "Videos", extensions: ["mp4", "webm", "mov", "mkv", "m4v"] }];
  const picked = await open({
    multiple: false,
    filters,
    title: kind === "image" ? "Choose a background photo" : "Choose a background video",
  });
  return typeof picked === "string" ? picked : null;
}

/**
 * Settings › Appearance.
 *
 * Writes the existing `AppSettings` fields. The parent persists the full object
 * through `settings_update` — this section does not add commands.
 */
export function AppearanceSection({
  draft,
  onChange,
}: {
  draft: AppSettings;
  onChange: (next: Partial<AppSettings>) => void;
}) {
  const theme = normalizeTheme(draft.theme);
  const opacityPct = Math.round(Math.min(1, Math.max(0, draft.uiBackgroundOpacity)) * 100);
  const blurPx = Math.min(40, Math.max(0, draft.uiBackgroundBlur));
  const kind = (
    draft.uiBackgroundKind === "image" || draft.uiBackgroundKind === "video"
      ? draft.uiBackgroundKind
      : "aurora"
  ) as "aurora" | "image" | "video";
  const [fontPct, setFontPct] = useState(() => Math.round((readFontScale() ?? 1) * 100));

  const onPickBackground = async (mode: "image" | "video") => {
    try {
      const path = await pickBackgroundFile(mode);
      if (!path) return;
      onChange({ uiBackgroundKind: mode, uiBackgroundPath: path });
    } catch {
      // Dialog unavailable in the browser preview, or cancelled.
    }
  };

  return (
    <>
      <Card>
        <CardHeader>
          <CardTitle>Theme presets</CardTitle>
        </CardHeader>
        <CardContent className="flex flex-col gap-3">
          <p className="text-xs leading-relaxed text-[var(--text-muted)]">
            Nebula Vault is the default. Accent chips still tint highlights on every preset.
          </p>
          <div className="grid grid-cols-2 gap-2.5 sm:grid-cols-3">
            {THEME_PRESETS.map((preset) => {
              const selected = theme === preset.id;
              return (
                <button
                  key={preset.id}
                  type="button"
                  onClick={() => onChange({ theme: preset.id })}
                  className={cn(
                    "rounded-[var(--radius-md)] border bg-[var(--surface-2)] p-3 text-left transition-colors",
                    selected
                      ? "border-[var(--rim-light)] shadow-[0_0_0_2px_var(--accent-dim)]"
                      : "border-[var(--border)] hover:border-[var(--border-strong)]",
                  )}
                >
                  <span className="mb-2 flex h-9 overflow-hidden rounded-md border border-white/6">
                    {preset.colors.map((color) => (
                      <i key={color} className="block flex-1" style={{ background: color }} />
                    ))}
                  </span>
                  <span className="block text-xs font-semibold">{preset.label}</span>
                  <span className="mt-0.5 block text-[11px] text-[var(--text-faint)]">{preset.hint}</span>
                </button>
              );
            })}
          </div>
        </CardContent>
      </Card>

      <Card>
        <CardHeader>
          <CardTitle>Accent</CardTitle>
        </CardHeader>
        <CardContent>
          <div className="flex flex-wrap items-center gap-3">
            {ACCENT_CHIPS.map((chip) => {
              const selected = draft.uiAccent === chip.id;
              return (
                <button
                  key={chip.id}
                  type="button"
                  title={chip.label}
                  aria-label={chip.label}
                  onClick={() => onChange({ uiAccent: chip.id, accent: chip.id })}
                  className={cn(
                    "size-7 rounded-full border-2 border-transparent transition-transform hover:scale-105",
                    selected && "shadow-[0_0_0_2px_var(--bg-void),0_0_0_4px_var(--accent-soft)]",
                  )}
                  style={{ background: chip.hex }}
                />
              );
            })}
          </div>
        </CardContent>
      </Card>

      <Card>
        <CardHeader>
          <CardTitle>Wallpaper</CardTitle>
        </CardHeader>
        <CardContent className="flex flex-col gap-4">
          <div className="grid gap-2.5 sm:grid-cols-3">
            {BG_MODES.map((mode) => {
              const selected = kind === mode.id;
              const Icon = mode.id === "image" ? ImageIcon : mode.id === "video" ? Video : Zap;
              return (
                <button
                  key={mode.id}
                  type="button"
                  onClick={() => {
                    if (mode.id === "aurora") {
                      onChange({ uiBackgroundKind: "aurora" });
                      return;
                    }
                    if (draft.uiBackgroundPath && draft.uiBackgroundKind === mode.id) return;
                    if (draft.uiBackgroundPath) {
                      onChange({ uiBackgroundKind: mode.id });
                      return;
                    }
                    void onPickBackground(mode.id);
                  }}
                  className={cn(
                    "rounded-[var(--radius-md)] border border-dashed p-4 text-left",
                    selected
                      ? "border-[var(--rim-light)] bg-[var(--accent-dim)]"
                      : "border-[var(--border-strong)] bg-[var(--surface-2)] hover:border-[var(--rim-light)]",
                  )}
                >
                  <Icon className="mb-2 size-6 text-[var(--accent-soft)]" strokeWidth={1.5} />
                  <div className="text-[13px] font-semibold">{mode.id === "image" ? "Photo" : mode.label}</div>
                  <div className="mt-0.5 text-[11px] text-[var(--text-faint)]">{mode.description}</div>
                </button>
              );
            })}
          </div>

          {kind !== "aurora" ? (
            <div className="flex flex-wrap items-center gap-2">
              <Badge variant="outline" className="max-w-full truncate font-normal tracking-normal normal-case">
                {draft.uiBackgroundPath ?? "No file selected"}
              </Badge>
              <Button size="sm" variant="secondary" onClick={() => void onPickBackground(kind)}>
                {kind === "video" ? "Upload video" : "Upload photo"}
              </Button>
              <Button
                size="sm"
                variant="ghost"
                onClick={() => onChange({ uiBackgroundKind: "aurora", uiBackgroundPath: null })}
              >
                Clear background
              </Button>
            </div>
          ) : null}

          <label className="grid grid-cols-[120px_1fr_48px] items-center gap-3 text-[13px] text-[var(--text-muted)]">
            Opacity
            <input
              type="range"
              min={0}
              max={100}
              step={1}
              value={opacityPct}
              disabled={kind === "aurora"}
              onChange={(event) => onChange({ uiBackgroundOpacity: Number(event.target.value) / 100 })}
              className="accent-[var(--accent)] disabled:opacity-40"
            />
            <span className="text-right font-mono text-xs text-[var(--accent-soft)]">{opacityPct}%</span>
          </label>
          <label className="grid grid-cols-[120px_1fr_48px] items-center gap-3 text-[13px] text-[var(--text-muted)]">
            Blur
            <input
              type="range"
              min={0}
              max={40}
              step={1}
              value={blurPx}
              disabled={kind === "aurora"}
              onChange={(event) => onChange({ uiBackgroundBlur: Number(event.target.value) })}
              className="accent-[var(--accent)] disabled:opacity-40"
            />
            <span className="text-right font-mono text-xs text-[var(--accent-soft)]">{blurPx}px</span>
          </label>
        </CardContent>
      </Card>

      <Card>
        <CardHeader>
          <CardTitle>Density and motion</CardTitle>
        </CardHeader>
        <CardContent className="flex flex-col gap-4">
          <div className="grid grid-cols-2 gap-2">
            <button
              type="button"
              onClick={() => onChange({ uiCompact: false })}
              className={cn(
                "rounded-[var(--radius-md)] border px-3 py-3 text-center text-xs font-medium",
                !draft.uiCompact
                  ? "border-[var(--rim-light)] bg-[var(--accent-dim)] text-[var(--text)]"
                  : "border-[var(--border)] bg-[var(--surface-2)] text-[var(--text-muted)]",
              )}
            >
              Comfortable
            </button>
            <button
              type="button"
              onClick={() => onChange({ uiCompact: true })}
              className={cn(
                "rounded-[var(--radius-md)] border px-3 py-3 text-center text-xs font-medium",
                draft.uiCompact
                  ? "border-[var(--rim-light)] bg-[var(--accent-dim)] text-[var(--text)]"
                  : "border-[var(--border)] bg-[var(--surface-2)] text-[var(--text-muted)]",
              )}
            >
              Compact
            </button>
          </div>
          <Field label={`Font scale · ${fontPct}%`} hint="Stored on this device. There is no settings field for it yet.">
            <input
              type="range"
              min={90}
              max={110}
              step={1}
              value={fontPct}
              onChange={(event) => {
                const next = Number(event.target.value);
                setFontPct(next);
                writeFontScale(next / 100);
              }}
              className="accent-[var(--accent)]"
            />
          </Field>
          <SettingRow
            title="UI animations"
            description="Page transitions and progress shimmer."
            control={
              <Switch checked={draft.uiAnimations} onCheckedChange={(value) => onChange({ uiAnimations: value })} />
            }
          />
          <SettingRow
            title="Reduce motion"
            description="Overrides animations for accessibility."
            control={
              <Switch checked={draft.reduceMotion} onCheckedChange={(value) => onChange({ reduceMotion: value })} />
            }
          />
        </CardContent>
      </Card>
    </>
  );
}
