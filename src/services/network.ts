/**
 * Network service — the Redis server browser and the P2P session pipeline.
 *
 * Hosting and joining are symmetric: `hostStart` publishes a listing and opens
 * a bridge, `joinServer` / `joinCode` open a local bridge the game connects to.
 * Both return a `JoinStatus`/`HostStatus` containing a *local* address, so the
 * game itself only ever talks to 127.0.0.1.
 */

import { call } from "./ipc";
import type {
  CachedServer,
  HostRequest,
  HostStatus,
  JoinRejection,
  JoinStatus,
  LanHost,
  LanHostRequest,
  LanReport,
  LanWorld,
  NatAdvice,
  NetworkStatus,
  ServerFilter,
  ServerListingSummary,
  SessionHistory,
} from "@/types/server";

export const networkService = {
  /** Browse the global directory (results are cached locally for offline use). */
  browse: (filter?: ServerFilter) => call<ServerListingSummary[]>("server_browse", { filter: filter ?? null }),

  /** Latency to a host; `null` for relayed sessions (latency is the relay's). */
  ping: (id: string) => call<number | null>("server_ping", { id }),
  favorites: () => call<CachedServer[]>("server_favorites"),

  setFavorite: (id: string, favorite: boolean) =>
    call<void>("server_set_favorite", { id, favorite }),

  status: () => call<NetworkStatus>("network_status"),

  /** One-click hosting: publishes the listing and returns a share code. */
  hostStart: (request: HostRequest) => call<HostStatus>("host_start", { request }),

  hostStop: (id: string) => call<void>("host_stop", { id }),

  hostStatus: (id: string) => call<HostStatus>("host_status", { id }),

  hostKick: (id: string, peerId: string, reason?: JoinRejection) =>
    call<boolean>("host_kick", { id, peerId, reason: reason ?? null }),

  /** Join by typing a share code — validated locally first. */
  joinCode: (code: string, instanceId?: string | null) =>
    call<JoinStatus>("join_code", { code, instanceId: instanceId ?? null }),

  joinServer: (id: string, instanceId?: string | null) =>
    call<JoinStatus>("join_server", { id, instanceId: instanceId ?? null }),

  leave: (id: string) => call<void>("leave_session", { id }),

  connectionCode: (id: string) => call<string>("connection_code", { id }),

  /** Round trip to a local bridge address (diagnostics). */
  localRtt: (address: string) => call<number>("local_rtt", { address }),

  /**
   * Browse the local network. No server of any kind is required: the launcher
   * broadcasts a probe and collects the answers. The receive loop also keeps a
   * live map, so `lanWorlds` below is the cheap polling path.
   */
  lanBrowse: (timeoutMs = 900) => call<LanReport>("lan_browse", { timeoutMs }),

  /** Worlds already heard, without sending a new probe. */
  lanWorlds: () => call<LanWorld[]>("lan_worlds"),

  /** Announce a world on the local network. */
  lanHostStart: (request: LanHostRequest) => call<LanHost>("lan_host_start", { request }),

  /** Stop announcing a world. */
  lanHostStop: (id: string) => call<boolean>("lan_host_stop", { id }),

  /** Announcement attached to an instance, if any. */
  lanHostForInstance: (instanceId: string) =>
    call<LanHost | null>("lan_host_for_instance", { instanceId }),

  /** Address friends should use for a hosted world. */
  lanAddress: () => call<string>("lan_address"),

  sessionHistory: (limit = 25) => call<SessionHistory>("session_history", { limit }),

  /** "Will hosting work from this network?" — STUN NAT classification. */
  natProbe: () => call<NatAdvice>("nat_probe"),
};

export type NetworkService = typeof networkService;

/**
 * Base32 length of a 21-byte `ConnectCode` payload: `ceil(21 * 8 / 5) = 34`.
 * Sixteen characters decode to 10 bytes, which the backend rejects.
 */
const SHARE_CODE_PAYLOAD_CHARS = 34;

/**
 * RFC 4648 base32, same set as `ALPHABET` in `src-tauri/src/network/code.rs`.
 * Digits are only 2–7. `0`, `1`, `8`, and `9` are never emitted.
 */
export const SHARE_CODE_ALPHABET = "ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";

/**
 * `SXM1-` plus eight groups of four and a final group of two.
 * Keep this in lockstep with `matches_share_code_pattern` on the Rust side.
 */
export const SHARE_CODE_PATTERN =
  /^SXM1(?:-[A-Z2-7]{4}){8}-[A-Z2-7]{2}$/;

/**
 * Payload characters after a leading `SXM1` prefix.
 *
 * Only a prefix is removed. A `1` later in the paste stays, so it can be
 * reported as invalid instead of disappearing into another `SXM1` strip.
 */
function shareCodePayload(raw: string): string {
  let cleaned = raw.toUpperCase().replace(/[^A-Z0-9]/g, "");
  while (cleaned.startsWith("SXM1")) cleaned = cleaned.slice(4);
  return cleaned.slice(0, SHARE_CODE_PAYLOAD_CHARS);
}

/**
 * Normalize a share code as the user types: groups of four, uppercase.
 */
export function formatShareCode(raw: string): string {
  const payload = shareCodePayload(raw);
  const groups = payload.match(/.{1,4}/g) ?? [];
  return ["SXM1", ...groups].join("-");
}

/** Characters in the payload that the encoder never emits (`0`, `1`, `8`, `9`, …). */
export function invalidShareCodeChars(raw: string): string[] {
  const seen: string[] = [];
  for (const character of shareCodePayload(raw)) {
    if (!SHARE_CODE_ALPHABET.includes(character) && !seen.includes(character)) {
      seen.push(character);
    }
  }
  return seen;
}

/** True when the code is a full 21-byte payload and can be submitted. */
export function isCompleteShareCode(code: string): boolean {
  return invalidShareCodeChars(code).length === 0 && SHARE_CODE_PATTERN.test(formatShareCode(code));
}
