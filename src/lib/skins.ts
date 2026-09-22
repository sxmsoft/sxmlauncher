/**
 * Skin and cape rendering.
 *
 * Four sources, in priority order:
 *   1. the texture URL the account already carries (Mojang for Microsoft,
 *      Ely.by for Ely.by accounts — sliced straight from the raw texture, so
 *      it needs no third party),
 *   2. `mc-heads.net` keyed by UUID or username (renders the default Steve/Alex
 *      skin for names that have no texture — this covers offline profiles and
 *      freshly changed skins),
 *   3. `crafatar` as a second render service (same idea, different host),
 *   4. a locally generated gradient avatar, so something is always shown —
 *      even with no network at all.
 *
 * Source order matters: it is why a signed-in account shows its actual skin,
 * an offline profile shows a default Steve instead of nothing, and an
 * air-gapped machine still renders a recognisable face.
 */

import type { CSSProperties } from "react";

import type { AccountSummary } from "@/types/account";

/** Undashed UUID when the account has one, else the username. */
export function identifierFor(account: AccountSummary): string {
  const uuid = account.uuid.replace(/-/g, "");
  if (uuid.replace(/^0+$/, "") !== "" && uuid.length === 32) return uuid;
  return account.username;
}

/** One rectangular region of the 64×64 texture, in texture pixels. */
interface Tile {
  x: number;
  y: number;
  w: number;
  h: number;
}

/**
 * X/Y tile (in texture pixels) of a body part face of the 64×64 skin layout.
 * The classic layout is shared by Steve, Alex and every custom skin.
 */
const FACE: { head: Tile; hat: Tile } = {
  head: { x: 8, y: 8, w: 8, h: 8 },
  hat: { x: 40, y: 8, w: 8, h: 8 },
};

function textureSlices(textureUrl: string, tile: Tile, scale: number) {
  return {
    backgroundImage: `url("${textureUrl}")`,
    backgroundSize: `${64 * scale}px ${64 * scale}px`,
    backgroundPosition: `-${tile.x * scale}px -${tile.y * scale}px`,
    width: tile.w * scale,
    height: tile.h * scale,
  } as const;
}

/** Square head render: hat layer over the face, sliced from a raw texture. */
export function headStyle(
  textureUrl: string | null,
  size = 128,
): { face: CSSProperties; hat: CSSProperties } | null {
  if (!textureUrl) return null;
  const scale = size / 8;
  return {
    face: textureSlices(textureUrl, FACE.head, scale),
    hat: textureSlices(textureUrl, FACE.hat, scale),
  };
}

/** mc-heads render services (UUID or username; default skin when unknown). */
const MC_HEADS = "https://mc-heads.net";

/** Square head render through a third-party service. */
export function headUrl(account: AccountSummary, size = 128): string {
  return `${MC_HEADS}/avatar/${identifierFor(account)}/${size}`;
}

/** mc-heads keeps the flat full-body render working for any account. */
export function headUrlFallback(account: AccountSummary, size = 128): string {
  return `https://crafatar.com/avatars/${identifierFor(account)}?size=${size}&overlay`;
}

/** Full-body render used by the skin preview. */
export function bodyUrl(account: AccountSummary, size = 320): string {
  return `${MC_HEADS}/body/${identifierFor(account)}/${size}`;
}

/** Full-body render through the fallback service. */
export function bodyUrlFallback(account: AccountSummary, size = 320): string {
  return `https://crafatar.com/renders/body/${identifierFor(account)}?size=${size}&overlay`;
}

/** Applied cape, when the account has one from a signed-in provider. */
export function capeUrl(account: AccountSummary): string | null {
  if (account.skin.capeUrl) return account.skin.capeUrl;
  if (account.provider === "offline" || account.provider === "ely_by") return null;
  return `${MC_HEADS}/capes/${identifierFor(account)}`;
}

/** Raw skin texture, for the flat head slice and the texture download. */
export function skinTextureUrl(account: AccountSummary): string | null {
  const url = account.skin.skinUrl;
  if (!url) return null;
  // Ely.by historically returns http://ely.by/storage/... — upgrade so the
  // webview can fetch the PNG without mixed-content / cleartext blocks.
  if (url.startsWith("http://ely.by/")) return `https://ely.by/${url.slice("http://ely.by/".length)}`;
  if (url.startsWith("http://skinsystem.ely.by/")) {
    return `https://skinsystem.ely.by/${url.slice("http://skinsystem.ely.by/".length)}`;
  }
  return url;
}

/**
 * Deterministic gradient for a username, used when no texture is available.
 * Same name always renders the same colours, so an offline profile looks stable.
 */
export function fallbackGradient(username: string): string {
  let hash = 0;
  for (const char of username) {
    hash = (hash * 31 + char.charCodeAt(0)) % 360;
  }
  const from = hash;
  const to = (hash + 48) % 360;
  return `linear-gradient(140deg, oklch(0.72 0.16 ${from}), oklch(0.52 0.18 ${to}))`;
}

/** Two-letter monogram for the fallback avatar. */
export function monogram(username: string): string {
  const trimmed = username.trim();
  if (trimmed.length === 0) return "??";
  if (trimmed.length === 1) return `${trimmed}${trimmed}`.toUpperCase();
  return `${trimmed.slice(0, 1)}${trimmed.slice(-1)}`.toUpperCase();
}

// ---------------------------------------------------------------------------
// Skin file inspection (used by the upload flow, picker and drop alike)
// ---------------------------------------------------------------------------

/** What the first bytes of an uploaded skin file say. */
export type PngHeader =
  | { kind: "not-png" }
  | { kind: "truncated" }
  | { kind: "png"; width: number; height: number };

/**
 * Read the PNG signature and IHDR dimensions from the head of a blob.
 * Deliberately discriminated: "not a PNG" and "PNG too short to measure"
 * deserve different sentences, and neither may throw.
 */
export async function readPngHeader(file: Blob): Promise<PngHeader> {
  const header = new Uint8Array(await file.slice(0, 24).arrayBuffer());
  const isPng =
    header[0] === 0x89 && header[1] === 0x50 && header[2] === 0x4e && header[3] === 0x47;
  if (!isPng) return { kind: "not-png" };
  // IHDR width/height live at bytes 16..24; shorter files are cut-off PNGs.
  if (header.length < 24) return { kind: "truncated" };
  const view = new DataView(header.buffer);
  return { kind: "png", width: view.getUint32(16), height: view.getUint32(20) };
}

/**
 * Integer downscale factor mapping an HD skin onto the standard 64×64 (or
 * legacy 64×32) canvas — 128×128 → 2, 512×512 → 8, 128×64 → 2. Null when the
 * size is not an HD skin layout: squashing an arbitrary 300×500 photo onto
 * 64×64 would produce garbage, so no offer is made for those.
 */
export function hdSkinScale(width: number, height: number): number | null {
  if (width < 128) return null;
  if (width % 64 !== 0) return null;
  if (width === height) return width / 64;
  if (height * 2 === width) return width / 64; // legacy 2:1 layout
  return null;
}

/**
 * Shrink an HD skin to its 64×64 / 64×32 equivalent in the browser.
 *
 * High-quality averaging is the right resampler for both source kinds: an
 * exact integer upscale of a 64px skin has uniform blocks, which average
 * back to the original pixels, while true HD art keeps as much detail as a
 * standard skin can carry. Returns null when decoding fails or the blob is
 * not an HD skin layout.
 */
export async function downscaleSkinTo64(file: Blob): Promise<Blob | null> {
  try {
    const bitmap = await createImageBitmap(file);
    try {
      const factor = hdSkinScale(bitmap.width, bitmap.height);
      if (!factor) return null;
      const canvas = document.createElement("canvas");
      canvas.width = 64;
      canvas.height = bitmap.height / factor;
      const ctx = canvas.getContext("2d");
      if (!ctx) return null;
      ctx.imageSmoothingEnabled = true;
      ctx.imageSmoothingQuality = "high";
      ctx.drawImage(bitmap, 0, 0, canvas.width, canvas.height);
      return await new Promise<Blob | null>((resolve) => canvas.toBlob(resolve, "image/png"));
    } finally {
      bitmap.close();
    }
  } catch {
    return null;
  }
}
