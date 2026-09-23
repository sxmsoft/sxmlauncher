/**
 * Discord Rich Presence copy for the current launcher screen.
 *
 * The Rust side only forwards these strings. Route changes, downloads, hosting,
 * and a running game all come through here so the status stays factual.
 */

export interface PresenceCopy {
  home: string;
  library: string;
  modpacks: string;
  profile: string;
  activity: string;
  settings: string;
  multiplayer: string;
  download: string;
  launching: string;
  hosting: string;
  joining: string;
  hostingShort: string;
  multiplayerShort: string;
  brand: string;
}

export interface PresenceInstance {
  id: string;
  name: string;
  gameVersion: string;
  loader: string;
}

export interface PresenceWork {
  kind: string;
  stage: string;
  label: string;
  finished: boolean;
  error: string | null;
  startedAtMs: number | null;
}

export interface PresenceInput {
  pathname: string;
  copy: PresenceCopy;
  instances: PresenceInstance[];
  runningIds: string[];
  /** Stable clock for the game that is open, unix milliseconds. */
  playingSinceMs: number | null;
  hostName: string | null;
  hostInstanceId: string | null;
  joinName: string | null;
  work: PresenceWork | null;
}

export interface PresencePayload {
  details: string;
  state: string;
  startUnixMs: number | null;
}

const LOADER_LABEL: Record<string, string> = {
  vanilla: "Vanilla",
  fabric: "Fabric",
  quilt: "Quilt",
  forge: "Forge",
  neoforge: "NeoForge",
};

const TRANSFER_KINDS = new Set([
  "instance_install",
  "modpack_install",
  "mod_download",
  "java_runtime",
  "asset_hydration",
  "launch",
]);

/** A job that should replace the page status until it finishes. */
export function isPresenceWork(job: PresenceWork): boolean {
  if (job.finished || job.error) return false;
  if (job.stage === "failed" || job.stage === "done") return false;
  if (job.kind === "p2p_host" || job.kind === "p2p_connect") {
    return job.stage === "connecting_p2p" || job.stage === "queued" || job.stage === "launching";
  }
  if (!TRANSFER_KINDS.has(job.kind)) return false;
  return job.stage !== "running";
}

function clip(value: string, fallback: string): string {
  const trimmed = value.replace(/\s+/g, " ").trim();
  const text = trimmed.length >= 2 ? trimmed : fallback;
  return text.length > 128 ? text.slice(0, 128) : text;
}

function loaderLabel(loader: string): string {
  const key = loader.trim().toLowerCase();
  return LOADER_LABEL[key] ?? (key ? key.charAt(0).toUpperCase() + key.slice(1) : "");
}

function routeKey(pathname: string): keyof Pick<
  PresenceCopy,
  "home" | "library" | "modpacks" | "profile" | "activity" | "settings" | "multiplayer"
> {
  const path = pathname.replace(/\/+$/, "") || "/";
  if (path === "/") return "home";
  if (path === "/library" || path.startsWith("/instances/")) return "library";
  if (path === "/modpacks" || path === "/custom-packs") return "modpacks";
  if (path === "/skin") return "profile";
  if (path === "/activity") return "activity";
  if (path === "/servers") return "multiplayer";
  if (path === "/settings") return "settings";
  return "home";
}

function playingState(copy: PresenceCopy, mode: "solo" | "host" | "join", loader: string, version: string): string {
  const parts = [loaderLabel(loader), version.trim()].filter((part) => part.length > 0);
  const spec = parts.join(" · ");
  if (mode === "host") return clip(`${copy.hostingShort} · ${spec || copy.brand}`, copy.brand);
  if (mode === "join") return clip(`${copy.multiplayerShort} · ${spec || copy.brand}`, copy.brand);
  return clip(spec, copy.brand);
}

/** Build the activity Discord should show for this moment. */
export function buildPresence(input: PresenceInput): PresencePayload {
  const { copy } = input;
  const running = input.instances.filter((instance) => input.runningIds.includes(instance.id));
  const playing =
    running.find((instance) => instance.id === input.hostInstanceId) ?? running[0] ?? null;

  if (playing) {
    const mode =
      input.hostInstanceId === playing.id ? "host" : input.joinName ? "join" : "solo";
    return {
      details: clip(playing.name, copy.brand),
      state: playingState(copy, mode, playing.loader, playing.gameVersion),
      startUnixMs: input.playingSinceMs,
    };
  }

  const work = input.work && isPresenceWork(input.work) ? input.work : null;
  if (work) {
    if (work.kind === "p2p_connect") {
      return {
        details: clip(copy.joining, copy.brand),
        state: clip(work.label, copy.brand),
        startUnixMs: work.startedAtMs,
      };
    }
    if (work.kind === "p2p_host") {
      return {
        details: clip(copy.hosting, copy.brand),
        state: clip(work.label, copy.brand),
        startUnixMs: work.startedAtMs,
      };
    }
    if (work.kind === "launch" || work.stage === "launching") {
      return {
        details: clip(copy.launching, copy.brand),
        state: clip(work.label, copy.brand),
        startUnixMs: work.startedAtMs,
      };
    }
    return {
      details: clip(copy.download, copy.brand),
      state: clip(work.label, copy.brand),
      startUnixMs: work.startedAtMs,
    };
  }

  if (input.joinName) {
    return {
      details: clip(copy.joining, copy.brand),
      state: clip(input.joinName, copy.brand),
      startUnixMs: null,
    };
  }

  if (input.hostName) {
    return {
      details: clip(copy.hosting, copy.brand),
      state: clip(input.hostName, copy.brand),
      startUnixMs: null,
    };
  }

  return {
    details: clip(copy[routeKey(input.pathname)], copy.brand),
    state: clip(copy.brand, "SXMLAUNCHER"),
    startUnixMs: null,
  };
}
