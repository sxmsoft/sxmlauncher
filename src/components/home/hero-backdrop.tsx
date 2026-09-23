/**
 * Original night scene for the Home hero. It is blurred on purpose so the
 * board reads as an immersive world, not a sharp illustration.
 */
export function HeroBackdrop() {
  return (
    <div className="pointer-events-none absolute inset-0 overflow-hidden" aria-hidden>
      <svg
        className="absolute inset-0 h-[118%] w-[118%] -translate-x-[6%] -translate-y-[4%] scale-105 blur-[10px]"
        viewBox="0 0 1280 720"
        preserveAspectRatio="xMidYMid slice"
      >
        <defs>
          <linearGradient id="sky" x1="0" y1="0" x2="0" y2="1">
            <stop offset="0%" stopColor="#1a1030" />
            <stop offset="55%" stopColor="#24324a" />
            <stop offset="100%" stopColor="#1c2a22" />
          </linearGradient>
        </defs>
        <rect width="1280" height="720" fill="url(#sky)" />
        <circle cx="980" cy="120" r="46" fill="#f4efe4" opacity="0.85" />
        <circle cx="220" cy="90" r="1.6" fill="#fff" />
        <circle cx="340" cy="150" r="1.2" fill="#fff" />
        <circle cx="510" cy="70" r="1.4" fill="#fff" />
        <circle cx="760" cy="160" r="1.2" fill="#fff" />
        <circle cx="1100" cy="200" r="1.5" fill="#fff" />
        <path fill="#2a3d34" d="M0 430h1280v290H0z" />
        <path fill="#3d6b3a" d="M0 390h180l40 40h120l-20 40H0z" />
        <path fill="#4e8a42" d="M160 360h200l30 50H220z" />
        <path fill="#3a6236" d="M420 400h260l40 30H380z" />
        <path fill="#5a9a48" d="M700 370h240l50 50H680z" />
        <path fill="#2f5530" d="M960 410h320v40H900z" />
        <path fill="#6b4a2a" d="M0 470h1280v80H0z" />
        <path fill="#3f6d38" d="M0 500h1280v220H0z" />
        <g fill="#1e3a24">
          <rect x="180" y="300" width="18" height="70" />
          <rect x="164" y="250" width="50" height="56" />
          <rect x="176" y="232" width="26" height="24" />
          <rect x="620" y="320" width="16" height="60" />
          <rect x="604" y="278" width="48" height="48" />
          <rect x="1040" y="330" width="16" height="80" />
          <rect x="1022" y="286" width="52" height="50" />
        </g>
        <rect x="300" y="520" width="48" height="48" fill="#8d6a3b" />
        <rect x="348" y="520" width="48" height="48" fill="#c2c6cc" />
        <rect x="396" y="472" width="48" height="48" fill="#9a9ea6" />
        <rect x="860" y="540" width="64" height="40" fill="#6e5433" />
      </svg>
      <div className="absolute inset-0 bg-gradient-to-t from-black/80 via-black/25 to-black/20" />
      <div className="absolute inset-0 bg-[radial-gradient(ellipse_at_center,transparent_20%,rgba(0,0,0,0.45)_100%)]" />
    </div>
  );
}
