/**
 * GenerateImage loader pack in `public/loaders`.
 *
 * Home and Library read these paths. Unknown loaders use the modpack plate
 * (or the charcoal underlay if that image is missing) — never a stock scene.
 */

export const LOADER_KEYS = ["vanilla", "fabric", "quilt", "forge", "neoforge", "modpack"] as const;

export type LoaderAssetKey = (typeof LOADER_KEYS)[number];

/** Charcoal underlay. Shown only when pack art cannot load. */
export const LOADER_FALLBACK_COLOR = "#141416";

export function loaderAssetKey(loader: string | null | undefined): LoaderAssetKey {
  const raw = String(loader ?? "")
    .trim()
    .toLowerCase()
    .replace(/[\s-]+/g, "_");
  if (raw === "neoforge" || raw === "neo_forge") return "neoforge";
  if (raw === "modpack" || raw === "pack") return "modpack";
  if (raw === "vanilla" || raw === "fabric" || raw === "quilt" || raw === "forge") return raw;
  return "modpack";
}

export function loaderBg(loader: string | null | undefined): string {
  return `/loaders/bg-${loaderAssetKey(loader)}.png`;
}

export function loaderIcon(loader: string | null | undefined, size: 64 | 256): string {
  return `/loaders/icon-${loaderAssetKey(loader)}-${size}.png`;
}
