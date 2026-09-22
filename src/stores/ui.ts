/**
 * UI state: toasts and the selections that outlive a route.
 *
 * Server state does **not** belong here — that is React Query's job. This store
 * holds only things that are genuinely client-side and need to be reachable from
 * outside the React tree (the imperative `toast.*` helpers).
 */

import { create } from "zustand";

import type { SettingsSection, ToastTone } from "@/types/system";

export interface Toast {
  id: string;
  tone: ToastTone;
  title: string;
  message?: string;
  /** Epoch ms — toasts auto-dismiss from the view layer. */
  createdAt: number;
}

interface UiState {
  toasts: Toast[];
  /** Instance the Dashboard acts on (Play button, host toggle). */
  selectedInstanceId: string | null;
  /** Deep link target for the Settings page. */
  settingsSection: SettingsSection;
  /** Custom pack the mod browser should offer to add into (deep link). */
  addPackTargetId: string | null;
  /** Activity slide-over with the Downloads/Running lists. */
  activityOpen: boolean;
  pushToast: (tone: ToastTone, title: string, message?: string) => string;
  dismissToast: (id: string) => void;
  selectInstance: (id: string | null) => void;
  openSettings: (section?: SettingsSection) => void;
  setAddPackTarget: (id: string | null) => void;
  setSettingsSection: (section: SettingsSection) => void;
  setActivityOpen: (open: boolean) => void;
}

const MAX_TOASTS = 4;

export const useUiStore = create<UiState>((set) => ({
  toasts: [],
  selectedInstanceId: null,
  settingsSection: "general",
  addPackTargetId: null,
  activityOpen: false,

  pushToast: (tone, title, message) => {
    const id = `t-${Date.now()}-${Math.random().toString(36).slice(2, 8)}`;
    set((state) => ({
      // Newest first, and never more than a handful on screen.
      toasts: [{ id, tone, title, message, createdAt: Date.now() }, ...state.toasts].slice(
        0,
        MAX_TOASTS,
      ),
    }));
    return id;
  },

  dismissToast: (id) =>
    set((state) => ({ toasts: state.toasts.filter((toast) => toast.id !== id) })),

  selectInstance: (id) => set({ selectedInstanceId: id }),

  openSettings: (section) =>
    set({ settingsSection: section ?? "general" }),

  setSettingsSection: (section) => set({ settingsSection: section }),

  setActivityOpen: (open) => set({ activityOpen: open }),
  setAddPackTarget: (id) => set({ addPackTargetId: id }),
}));

/**
 * Imperative toast helpers, for use inside async event handlers and stores.
 *
 * `toast.error(e)` understands the backend's typed failures, so an unreachable
 * Redis directory or a missing sign-in reads as a sentence instead of a code.
 */
export const toast = {
  info: (title: string, message?: string) =>
    useUiStore.getState().pushToast("info", title, message),
  success: (title: string, message?: string) =>
    useUiStore.getState().pushToast("success", title, message),
  warning: (title: string, message?: string) =>
    useUiStore.getState().pushToast("warning", title, message),
  error: (error: unknown, title = "Something went wrong") => {
    const message = error instanceof Error ? error.message : String(error);
    return useUiStore.getState().pushToast("error", title, message);
  },
  dismiss: (id: string) => useUiStore.getState().dismissToast(id),
};
