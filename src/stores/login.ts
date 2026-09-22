/**
 * Sign-in flow state.
 *
 * A browser sign-in is a two-call dance: `begin_login` parks a loopback listener
 * in the backend and returns a URL, then `complete_login` waits for the redirect.
 * The pending attempt therefore has to live somewhere that survives the dialog
 * being re-rendered — that is this store. Cancelling must always be possible, or
 * the listener would hold its port until the app exits.
 */

import { create } from "zustand";

import { accountService } from "@/services";
import type { AccountProvider } from "@/types/account";
import type { DeviceCodePrompt } from "@/types/account";

interface LoginState {
  /** The attempt we have not completed yet, if any. */
  pending: {
    loginId: string;
    provider: AccountProvider;
    authorizeUrl: string;
    redirectUri: string;
  } | null;
  /** Device-code prompt, when that flow is in flight. */
  deviceCode: DeviceCodePrompt | null;
  /** True while a command is outstanding. */
  busy: boolean;
  /** Human-readable failure from the last attempt. */
  error: string | null;
  setPending: (pending: LoginState["pending"]) => void;
  setDeviceCode: (prompt: DeviceCodePrompt | null) => void;
  setBusy: (busy: boolean) => void;
  setError: (error: string | null) => void;
  /** Release the loopback listener of the current attempt. */
  cancel: () => Promise<void>;
  reset: () => void;
}

export const useLoginStore = create<LoginState>((set, get) => ({
  pending: null,
  deviceCode: null,
  busy: false,
  error: null,

  setPending: (pending) => set({ pending }),
  setDeviceCode: (deviceCode) => set({ deviceCode }),
  setBusy: (busy) => set({ busy }),
  setError: (error) => set({ error }),

  cancel: async () => {
    const { pending } = get();
    set({ pending: null, error: null });
    if (!pending) return;
    try {
      await accountService.cancelLogin(pending.loginId);
    } catch {
      // Cancelling is best-effort: the listener is also released on app exit.
    }
  },

  reset: () => set({ pending: null, deviceCode: null, busy: false, error: null }),
}));
