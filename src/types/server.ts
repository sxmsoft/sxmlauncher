/**
 * The Redis global server browser payload — mirrors
 * `src-tauri/src/models/server.rs` field for field.
 *
 * This is a **wire contract**, not an internal type: any client (this app, a web
 * browser, a Discord bot) can read `sxml:servers:{id}` and render it. Bump
 * `SERVER_LISTING_SCHEMA` in Rust when a field changes meaning.
 */

import type { LoaderKind } from "./instance";
import type { ModSource } from "./modpack";

export const SERVER_LISTING_SCHEMA = 1;
export const DEFAULT_HEARTBEAT_TTL_SECS = 30;

export type ConnectionMode = "direct_p2p" | "relay" | "dedicated";

export type EndpointKind = "public" | "local" | "relay";

export interface PeerEndpoint {
  kind: EndpointKind;
  /** `ip:port` (IPv4 for direct codes). */
  addr: string;
  expiresAt: string | null;
}

export interface RelayDescriptor {
  url: string;
  roomToken: string;
  region: string | null;
  certFingerprint: string | null;
}

/** Everything a guest needs to attempt a connection. */
export interface ConnectionDescriptor {
  mode: ConnectionMode;
  peerId: string;
  publicKey: string;
  endpoints: PeerEndpoint[];
  relay: RelayDescriptor | null;
  sessionToken: string;
  protocolVersion: number;
}

export interface PlayerCount {
  online: number;
  max: number;
}

export interface ServerOwner {
  name: string;
  uuid: string;
  provider: "microsoft" | "ely_by" | "offline" | "sx_acc";
}

export interface ModpackRef {
  source: ModSource;
  projectId: string;
  versionId: string;
  name: string;
  versionNumber: string;
}

export interface WhitelistPolicy {
  enabled: boolean;
  allowedUuids: string[];
}

/** The document published to Redis (and cached in SQLite). */
export interface ServerListing {
  schemaVersion: number;
  id: string;
  name: string;
  description: string;
  motd: string;
  /** Base64 PNG/JPEG, capped at 64 KiB. */
  iconBase64: string | null;
  owner: ServerOwner;
  gameVersion: string;
  loader: LoaderKind;
  loaderVersion: string | null;
  modpack: ModpackRef | null;
  requiredModIds: string[];
  players: PlayerCount;
  connection: ConnectionDescriptor;
  region: string | null;
  tags: string[];
  whitelist: WhitelistPolicy;
  passwordProtected: boolean;
  worldName: string | null;
  createdAt: string;
  heartbeatAt: string;
  ttlSecs: number;
  worldPlaytimeSecs: number;
}

/** Card payload returned by `server_browse`. */
export interface ServerListingSummary {
  id: string;
  name: string;
  description: string;
  iconBase64: string | null;
  ownerName: string;
  gameVersion: string;
  loader: LoaderKind;
  modpackName: string | null;
  players: PlayerCount;
  mode: ConnectionMode;
  region: string | null;
  tags: string[];
  passwordProtected: boolean;
  heartbeatAt: string;
  /** Milliseconds since the last heartbeat (freshness badge). */
  ageMs: number;
  pingMs: number | null;
  versionMismatch: boolean;
}

export interface ServerFilter {
  query?: string;
  gameVersion?: string;
  loader?: LoaderKind;
  modpackProjectId?: string;
  tags?: string[];
  hideFull?: boolean;
  hidePasswordProtected?: boolean;
  maxPingMs?: number;
  limit?: number;
}

/** Reasons a host can refuse a join. */
export type JoinRejection =
  | "server_full"
  | "not_whitelisted"
  | "wrong_password"
  | "version_mismatch"
  | "missing_mods"
  | "host_closed"
  | "banned";

/** Request body for `host_start`. */
export interface HostRequest {
  instanceId?: string;
  name: string;
  description?: string;
  motd?: string;
  iconBase64?: string;
  maxPlayers?: number;
  password?: string;
  whitelist?: WhitelistPolicy;
  public?: boolean;
  worldName?: string;
  tags?: string[];
  /** Local Minecraft server port to expose (default 25565). */
  localPort?: number;
  forceRelay?: boolean;
}

export interface GuestStatus {
  peerId: string;
  address: string;
  username: string | null;
  protocolVersion: number | null;
  joinedAt: string;
}

export interface HostStatus {
  id: string;
  /** Shareable code, e.g. `SXM1-AEAI-RMQG-NTKC-WSQT-ELAX-3T6N-CYAB-FF6E-NQ`. */
  shareCode: string;
  summary: ServerListingSummary;
  mode: ConnectionMode;
  /** Plain-English NAT verdict shown in the hosting panel. */
  natAdvice: string;
  publicEndpoint: string | null;
  guests: GuestStatus[];
}

export interface JoinStatus {
  id: string;
  serverName: string;
  mode: ConnectionMode;
  /** Address to pass to the game as `--server` / `--port`. */
  localAddress: string;
  localPort: number;
  remote: string | null;
  rttMs: number | null;
}

export interface NetworkStatus {
  directoryConnected: boolean;
  directoryUrl: string;
  onlinePlayers: number;
  activeHosts: number;
  activeGuests: number;
  relayConfigured: boolean;
  message: string | null;
  /** LAN needs no directory and is always available. */
  lanEnabled: boolean;
  /** UDP port used for LAN beacons. */
  lanPort: number;
  /** Worlds heard on the local network right now. */
  lanWorlds: number;
}

export interface CachedServer {
  listing: ServerListing;
  favorite: boolean;
  lastSeenAt: string;
}

export interface NatAdvice {
  behavior: "endpoint_independent" | "address_dependent" | "unknown";
  publicAddress: string | null;
  canHostDirect: boolean;
  message: string;
}

export interface JoinHistoryEntry {
  serverId: string | null;
  serverName: string;
  instanceId: string | null;
  mode: ConnectionMode | null;
  joinedAt: string;
  outcome: string;
  detail: string | null;
}

export interface P2pSessionRecord {
  id: string;
  role: string;
  peerId: string;
  mode: ConnectionMode;
  localPort: number | null;
  startedAt: string;
  endedAt: string | null;
  bytesUp: number;
  bytesDown: number;
  rttMs: number | null;
  detail: string | null;
}

export interface SessionHistory {
  joins: JoinHistoryEntry[];
  p2p: P2pSessionRecord[];
  /** Share of recent sessions that needed the relay. */
  relayRatio: number;
}

/** A world discovered on the local network (mirrors `LanWorld` in the backend). */
export interface LanWorld {
  id: string;
  name: string;
  motd: string;
  host: string;
  /** Address the game connects to (`--server`). */
  address: string;
  /** Port the game connects to (`--port`). */
  port: number;
  gameVersion: string;
  loader: string;
  players: number;
  maxPlayers: number;
  passwordProtected: boolean;
  protocolVersion: number;
  tags: string[];
  instanceId: string | null;
  worldName: string | null;
  /** Seconds since the last beacon — the UI greys out stale entries. */
  lastSeenSecs: number;
}

/** A world this launcher is announcing on the LAN. */
export interface LanHost {
  id: string;
  name: string;
  port: number;
  /** The address friends on the same network should use. */
  address: string;
  instanceId: string | null;
  worldName: string | null;
  gameVersion: string;
  players: number;
  maxPlayers: number;
  /** `true` when the port came from the game log, not a manual entry. */
  autoDetected: boolean;
}

/** Everything the LAN tab needs in one round trip. */
export interface LanReport {
  enabled: boolean;
  /** UDP port used for beacons. */
  port: number;
  /** Worlds other machines on this network are serving. */
  worlds: LanWorld[];
  /** Worlds this machine is announcing. */
  hosting: LanHost[];
  /** Ready-to-share instruction line for the UI. */
  hint: string;
}

/** Request body for `lan_host_start`. */
export interface LanHostRequest {
  name?: string;
  motd?: string;
  /** Port the world listens on. */
  port: number;
  instanceId?: string | null;
  worldName?: string | null;
  gameVersion?: string;
  loader?: string;
  players?: number;
  maxPlayers?: number;
  passwordProtected?: boolean;
  tags?: string[];
}

/** Payload emitted on `session://event`. */
export interface SessionEventPayload {
  id: string;
  kind: string;
  peerId: string | null;
  username: string | null;
  players: PlayerCount | null;
  message: string | null;
}

/**
 * Collapse a full listing into the summary a browser card shows.
 *
 * The directory already sends summaries for live results; this exists for the
 * *cached* listings (favourites, join history), which are stored whole so they
 * keep working offline.
 */
export function summarizeListing(listing: ServerListing): ServerListingSummary {
  const heartbeat = new Date(listing.heartbeatAt).getTime();
  return {
    id: listing.id,
    name: listing.name,
    description: listing.description,
    iconBase64: listing.iconBase64,
    ownerName: listing.owner.name,
    gameVersion: listing.gameVersion,
    loader: listing.loader,
    modpackName: listing.modpack?.name ?? null,
    players: listing.players,
    mode: listing.connection.mode,
    region: listing.region,
    tags: listing.tags,
    passwordProtected: listing.passwordProtected,
    heartbeatAt: listing.heartbeatAt,
    ageMs: Number.isNaN(heartbeat) ? Number.MAX_SAFE_INTEGER : Date.now() - heartbeat,
    pingMs: null,
    versionMismatch: false,
  };
}

/** Freshness of a listing, used for the "live" dot. */
export function isFresh(summary: ServerListingSummary): boolean {
  return summary.ageMs < DEFAULT_HEARTBEAT_TTL_SECS * 1000;
}

export function freeSlots(players: PlayerCount): number {
  return Math.max(0, players.max - players.online);
}
