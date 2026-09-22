/**
 * Account service — the three signing-in providers.
 *
 * Tokens never cross this boundary: `begin_login` hands back a URL to open and
 * a `loginId` to complete with, and every account object the UI sees is a
 * `AccountSummary` (no access or refresh token, ever).
 */

import { call } from "./ipc";
import type {
  AccountProvider,
  AccountSummary,
  DeviceCodePrompt,
  LaunchIdentity,
  PendingLoginInfo,
  SkinModel,
  SkinUploadOutcome,
} from "@/types/account";

export const accountService = {
  list: () => call<AccountSummary[]>("account_list"),

  active: () => call<AccountSummary | null>("account_active"),

  setActive: (id: string) => call<AccountSummary>("account_set_active", { id }),

  /** Guest mode: a local profile with a deterministic `OfflinePlayer:<name>` UUID. */
  loginOffline: (username: string) =>
    call<AccountSummary>("account_login_offline", { username }),

  /** Start the OAuth2 + PKCE browser flow (Microsoft or Ely.by). */
  beginLogin: (provider: AccountProvider) =>
    call<PendingLoginInfo>("account_begin_login", { provider }),

  /** Wait for the loopback redirect and exchange the code for a session. */
  completeLogin: (loginId: string) =>
    call<AccountSummary>("account_complete_login", { loginId }),

  /** Abandon a pending sign-in and release its loopback port. */
  cancelLogin: (loginId: string) =>
    call<boolean>("account_cancel_login", { loginId }),

  /** Ely.by's Authlib endpoint accepts a direct username/password pair. */
  loginElybyPassword: (username: string, password: string) =>
    call<AccountSummary>("account_login_elyby_password", { username, password }),

  /** Device-code grant, for machines where opening a browser is not possible. */
  beginDeviceCode: () => call<DeviceCodePrompt>("account_begin_device_code"),

  completeDeviceCode: (prompt: DeviceCodePrompt) =>
    call<AccountSummary>("account_complete_device_code", { prompt }),

  refresh: (id: string) => call<AccountSummary>("account_refresh", { id }),

  /** Re-read an account's skin/cape from its provider and persist it. */
  refreshSkin: (id: string) =>
    call<import("@/types/account").SkinProfile>("account_refresh_skin", { id }),

  /**
   * Apply a picked 64×64 PNG to the account. Microsoft: a real upload via the
   * official Mojang endpoint; Ely.by: validated locally, returns a deep link
   * to the website's skin page (its public API has no upload endpoint);
   * offline: rejected backend-side.
   */
  uploadSkin: (id: string, model: SkinModel, png: Uint8Array) =>
    call<SkinUploadOutcome>("account_upload_skin", {
      id,
      model,
      png: Array.from(png),
    }),

  signOut: (id: string) => call<void>("account_sign_out", { id }),

  /** Remove the vault entry but keep the account row. */
  forgetCredentials: (id: string) => call<void>("account_forget_credentials", { id }),

  /** Build a launch identity without launching (diagnostics pane). */
  launchIdentity: (id: string) => call<LaunchIdentity>("account_launch_identity", { id }),

  /** Which OS credential store backs the token vault. */
  vaultBackend: () => call<string>("account_vault_backend"),
};

export type AccountService = typeof accountService;
