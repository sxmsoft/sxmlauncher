/**
 * Which crystalline mark is on screen.
 *
 * `applyAppearance` writes this whenever `--accent` changes, so the sidebar
 * and header swap PNGs in the same turn as the buttons.
 */

import { create } from "zustand";

import type { AccentId } from "@/lib/appearance";

interface AccentMarkState {
  mark: AccentId;
  setMark: (mark: AccentId) => void;
}

export const useAccentMarkStore = create<AccentMarkState>((set) => ({
  mark: "purple",
  setMark: (mark) => set({ mark }),
}));
