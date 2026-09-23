/**
 * Product mark. Every filled shape uses `var(--accent)`, so a theme change
 * tints the logo without swapping assets.
 *
 * `data-brand-slot="mark"` is the hook a later pass can use to drop in
 * per-color SVG variants. Until then the same paths follow `--accent`.
 */
export function BrandMark({ className }: { className?: string }) {
  return (
    <svg
      viewBox="0 0 32 32"
      className={className}
      aria-hidden
      data-brand-slot="mark"
      data-accent-hook="var(--accent)"
    >
      <path
        fill="var(--accent)"
        d="M16 2.2 28.8 9.4v13.2L16 29.8 3.2 22.6V9.4L16 2.2Zm0 3.6L6.4 11v10l9.6 5.2L25.6 21V11L16 5.8Z"
      />
      <path fill="var(--accent)" d="M16 10.4 21.2 16 16 21.6 10.8 16 16 10.4Z" />
    </svg>
  );
}
