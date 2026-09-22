/**
 * Account, session and identity interfaces.
 *
 * Mirrors `src-tauri/src/models/account.rs` exactly: the backend serializes with
 * `rename_all = "camelCase"`, so these field names are the wire format. Nothing
 * secret ever appears here — access and refresh tokens stay in the Rust process
 * and the OS credential vault.
 */

export type AccountProvider = "microsoft" | "ely_by" | "offline";

export type SkinModel = "classic" | "slim";

export interface SkinProfile {
  model: SkinModel;
  skinUrl: string | null;
  capeUrl: string | null;
}

/** An account as the UI sees it (never contains tokens). */
export interface AccountSummary {
  id: string;
  provider: AccountProvider;
  username: string;
  /** Dashed Minecraft UUID. */
  uuid: string;
  skin: SkinProfile;
  /** True when the OS vault holds a refresh token for this account. */
  hasStoredCredentials: boolean;
  expiresAt: string | null;
  lastUsedAt: string;
}

/** What `account_upload_skin` returns. */
export interface SkinUploadOutcome {
  /** `true` when the provider actually received the texture (Microsoft). */
  uploaded: boolean;
  /** Fresh profile read back after the upload (unchanged for Ely.by). */
  skin: SkinProfile;
  /** Human follow-up the UI should surface. */
  message: string | null;
  /** Deep link the UI may open in a browser (Ely.by skin page). */
  url: string | null;
}

/** `userType` values Mojang's session server understands. */
export type UserType = "msa" | "legacy" | "mojang";

export interface LaunchIdentity {
  username: string;
  /** Undashed UUID, as the launch arguments expect. */
  uuid: string;
  accessToken: string;
  userType: UserType;
  xuid: string | null;
  clientId: string | null;
  offline: boolean;
  /** Authlib-injector endpoint (Ely.by); must be attached to the JVM args. */
  authlibUrl: string | null;
}

/** Returned by `account_begin_login` to drive the browser flow. */
export interface PendingLoginInfo {
  loginId: string;
  provider: AccountProvider;
  authorizeUrl: string;
  redirectUri: string;
}

/** Device-code grant prompt (headless / no-browser sign-in). */
export interface DeviceCodePrompt {
  userCode: string;
  verificationUri: string;
  message: string;
  expiresIn: number;
  interval: number;
  deviceCode: string;
}

export interface AccountCreateOfflineRequest {
  username: string;
}

export interface ElyByPasswordLogin {
  username: string;
  password: string;
}

/** Human label for a provider, used in menus and badges. */
export const PROVIDER_LABEL: Record<AccountProvider, string> = {
  microsoft: "Microsoft",
  ely_by: "Ely.by",
  offline: "Offline",
};

/** Whether a provider can be refreshed without a new sign-in. */
export function providerIsOnline(provider: AccountProvider): boolean {
  return provider !== "offline";
}

/** True when the stored session is expired (or about to be). */
export function sessionExpired(
  account: AccountSummary,
  skewSeconds = 120,
): boolean {
  if (!account.expiresAt) return false;
  const expiresAt = new Date(account.expiresAt).getTime();
  return expiresAt - skewSeconds * 1000 <= Date.now();
}
