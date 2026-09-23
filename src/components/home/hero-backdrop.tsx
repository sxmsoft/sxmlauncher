import { LOADER_FALLBACK_COLOR, loaderBg } from "@/lib/loader-assets";

/**
 * Blurred loader plate for the Home hero, with a glass wash so Play stays readable.
 * The charcoal fill is only the underlay if the plate fails to load.
 */
export function HeroBackdrop({ loader }: { loader: string }) {
  return (
    <div
      className="pointer-events-none absolute inset-0 overflow-hidden"
      style={{ background: LOADER_FALLBACK_COLOR }}
      aria-hidden
    >
      <img
        src={loaderBg(loader)}
        alt=""
        className="absolute inset-0 h-full w-full scale-105 object-cover blur-[8px] brightness-125"
      />
      <div className="absolute inset-0 bg-[linear-gradient(180deg,rgba(255,255,255,0.07),transparent_32%)]" />
      <div className="absolute inset-0 bg-gradient-to-t from-black/75 via-black/15 to-black/10" />
    </div>
  );
}
