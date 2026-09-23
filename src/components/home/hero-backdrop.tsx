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
        className="absolute inset-0 h-[118%] w-[118%] -translate-x-[6%] -translate-y-[4%] scale-105 object-cover blur-[12px]"
      />
      <div className="absolute inset-0 bg-white/[0.04] backdrop-blur-[2px]" />
      <div className="absolute inset-0 bg-gradient-to-t from-black/80 via-black/30 to-black/25" />
      <div className="absolute inset-0 bg-[radial-gradient(ellipse_at_center,transparent_18%,rgba(0,0,0,0.42)_100%)]" />
    </div>
  );
}
