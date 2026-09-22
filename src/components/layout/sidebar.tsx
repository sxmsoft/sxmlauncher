import { NavLink } from "react-router-dom";

import { Blocks, CirclePlay, Globe, Package, Palette, Settings } from "lucide-react";

import { useLanWorlds, useNetworkStatus } from "@/hooks/queries";
import { cn } from "@/lib/utils";
import { useJobsStore } from "@/stores/jobs";

interface NavItem {
  to: string;
  label: string;
  icon: typeof CirclePlay;
  hint: string;
}

/**
 * Icon rail. Destinations stay on the existing routes: Play holds instances
 * and launch, Worlds is the host screen, packs and profile keep their pages.
 */
const NAV: NavItem[] = [
  { to: "/", label: "Play", icon: CirclePlay, hint: "Launch and instances" },
  { to: "/modpacks", label: "Packs", icon: Package, hint: "Modrinth and CurseForge" },
  { to: "/custom-packs", label: "Custom", icon: Blocks, hint: "Build your own modpacks" },
  { to: "/servers", label: "Host", icon: Globe, hint: "Worlds, LAN, and join codes" },
];

const FOOT: NavItem[] = [
  { to: "/settings", label: "Settings", icon: Settings, hint: "Appearance, Java, network" },
  { to: "/skin", label: "Profile", icon: Palette, hint: "Account, skin and cape" },
];

export function Sidebar() {
  const { data: status } = useNetworkStatus();
  const { data: lanWorlds } = useLanWorlds();
  const activeJobCount = useJobsStore(
    (state) => state.jobs.filter((job) => !job.finished && !job.error).length,
  );

  const badgeFor = (to: string): string | null => {
    if (to === "/servers") {
      const live = (lanWorlds?.length ?? 0) + (status?.onlinePlayers ?? 0);
      return live > 0 ? String(live) : null;
    }
    if (to === "/" && activeJobCount > 0) return String(activeJobCount);
    return null;
  };

  return (
    <nav
      aria-label="Primary"
      className="no-drag z-20 flex shrink-0 flex-col items-center gap-1 border-r border-[var(--border)] px-2.5 py-4"
      style={{
        width: "var(--nav-rail)",
        background: "color-mix(in srgb, var(--surface-1) 88%, transparent)",
        backdropFilter: "blur(var(--blur-panel))",
      }}
    >
      <div
        className="mb-5 grid size-10 place-items-center rounded-[11px] text-[13px] font-bold tracking-tight text-white"
        style={{
          background: "linear-gradient(145deg, var(--accent-soft), var(--accent))",
          boxShadow: "0 0 20px var(--accent-dim), inset 0 1px 0 rgba(255,255,255,0.25)",
        }}
        title="SXMLAUNCHER"
      >
        SX
      </div>

      {NAV.map((item) => (
        <RailLink key={item.to} item={item} badge={badgeFor(item.to)} />
      ))}

      <div className="flex-1" />

      {FOOT.map((item) => (
        <RailLink key={item.to} item={item} badge={null} />
      ))}
    </nav>
  );
}

function RailLink({ item, badge }: { item: NavItem; badge: string | null }) {
  return (
    <NavLink to={item.to} end={item.to === "/"} title={item.hint} className="w-full">
      {({ isActive }) => (
        <span
          className={cn(
            "relative flex w-full flex-col items-center gap-1 rounded-[var(--radius-md)] px-1.5 py-2.5 text-[var(--text-muted)] transition-colors",
            isActive
              ? "bg-[var(--accent-dim)] text-[var(--text)] shadow-[inset_0_0_0_1px_var(--border)]"
              : "hover:bg-[var(--accent-dim)] hover:text-[var(--text)]",
          )}
        >
          {isActive ? (
            <span
              className="absolute top-1/2 -left-2.5 h-[22px] w-[3px] -translate-y-1/2 rounded-r-[3px] bg-[var(--accent-soft)]"
              style={{ boxShadow: "0 0 10px var(--accent-glow)" }}
              aria-hidden
            />
          ) : null}
          <item.icon className="size-[22px]" strokeWidth={1.75} aria-hidden />
          <span className="text-center text-[9px] leading-tight font-medium">{item.label}</span>
          {badge ? (
            <span className="font-mono absolute top-1 right-1 text-[9px] text-[var(--accent-soft)]">
              {badge}
            </span>
          ) : null}
        </span>
      )}
    </NavLink>
  );
}
