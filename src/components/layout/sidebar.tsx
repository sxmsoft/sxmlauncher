import { NavLink, useLocation } from "react-router-dom";

import { Compass, House, Library, Radio, Settings, SquareActivity, UserRound } from "lucide-react";
import { useTranslation } from "react-i18next";

import { BrandMark } from "@/components/brand/logo";
import { useLanWorlds, useNetworkStatus } from "@/hooks/queries";
import { cn } from "@/lib/utils";
import { useJobsStore } from "@/stores/jobs";

interface NavItem {
  to: string;
  labelKey: string;
  hintKey: string;
  icon: typeof House;
}

/**
 * Narrow icon rail. Destinations are the launcher's own sections:
 * Home, Library, Browse, Host, Activity, Profile, Settings.
 */
const NAV: NavItem[] = [
  { to: "/", labelKey: "nav.home", hintKey: "nav.homeHint", icon: House },
  { to: "/library", labelKey: "nav.library", hintKey: "nav.libraryHint", icon: Library },
  { to: "/modpacks", labelKey: "nav.browse", hintKey: "nav.browseHint", icon: Compass },
  { to: "/servers", labelKey: "nav.host", hintKey: "nav.hostHint", icon: Radio },
  { to: "/activity", labelKey: "nav.activity", hintKey: "nav.activityHint", icon: SquareActivity },
];

const FOOT: NavItem[] = [
  { to: "/skin", labelKey: "nav.profile", hintKey: "nav.profileHint", icon: UserRound },
  { to: "/settings", labelKey: "nav.settings", hintKey: "nav.settingsHint", icon: Settings },
];

export function Sidebar() {
  const { t } = useTranslation();
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
    if (to === "/activity" && activeJobCount > 0) return String(activeJobCount);
    return null;
  };

  return (
    <nav
      aria-label={t("nav.primary")}
      className="no-drag z-20 flex shrink-0 flex-col items-center gap-1 border-r border-white/8 px-2 py-3"
      style={{
        width: "var(--nav-rail)",
        background: "color-mix(in srgb, #121214 88%, transparent)",
        backdropFilter: "blur(var(--blur-panel))",
      }}
    >
      <div className="mb-3 grid size-10 place-items-center" title="SXMLAUNCHER">
        <BrandMark className="size-7" />
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
  const { t } = useTranslation();
  const location = useLocation();
  const label = t(item.labelKey);

  return (
    <NavLink to={item.to} end={item.to === "/"} title={t(item.hintKey)} className="w-full">
      {({ isActive }) => {
        const active = isActive || (item.to !== "/" && location.pathname.startsWith(item.to));
        return (
          <span
            className={cn(
              "relative flex w-full flex-col items-center gap-1 rounded-2xl px-1 py-2 text-[10px] leading-tight font-medium text-[var(--text-faint)] transition-colors",
              active
                ? "bg-[var(--accent-dim)] text-[var(--accent-soft)] shadow-[inset_0_0_0_1px_color-mix(in_srgb,var(--accent)_35%,transparent)]"
                : "hover:bg-white/5 hover:text-[var(--text)]",
            )}
          >
            <item.icon
              className={cn("size-[18px]", active && "text-[var(--accent)]")}
              strokeWidth={1.5}
              aria-hidden
            />
            <span className="max-w-full truncate text-center tracking-tight">{label}</span>
            {badge ? (
              <span className="absolute top-1 right-1 font-mono text-[9px] text-[var(--accent)]">{badge}</span>
            ) : null}
          </span>
        );
      }}
    </NavLink>
  );
}
