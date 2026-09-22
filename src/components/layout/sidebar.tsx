import { NavLink } from "react-router-dom";

import { Activity, Blocks, CirclePlay, Globe, Package, Palette, Settings, UserPlus } from "lucide-react";

import { AccountMenu } from "@/components/account/account-menu";
import { Badge, StatusDot } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Hint } from "@/components/ui/tooltip";
import { useActiveAccount, useLanWorlds, useNetworkStatus } from "@/hooks/queries";
import { cn } from "@/lib/utils";
import { BROWSER_MODE } from "@/services";
import { useJobsStore } from "@/stores/jobs";
import { useSessionsStore } from "@/stores/sessions";
import { useUiStore } from "@/stores/ui";

interface NavItem {
  to: string;
  label: string;
  icon: typeof CirclePlay;
  hint: string;
}

/**
 * Four destinations instead of six: Play (instances + launch + LAN hosting),
 * Worlds (the server browser, LAN first), Modpacks (Modrinth/CurseForge),
 * Profile & Skin merged into Appearance. Settings keeps everything else.
 *
 * Fewer choices is the point: every screen the players used daily is now one
 * click away, and the rest lives inside Settings' left rail.
 */
const NAV: NavItem[] = [
  { to: "/", label: "Play", icon: CirclePlay, hint: "Instances, launch, host on LAN" },
  { to: "/servers", label: "Worlds", icon: Globe, hint: "LAN worlds + global P2P browser" },
  { to: "/modpacks", label: "Modpacks", icon: Package, hint: "Modrinth & CurseForge" },
  { to: "/custom-packs", label: "Custom packs", icon: Blocks, hint: "Build and play your own modpacks" },
  { to: "/skin", label: "Profile", icon: Palette, hint: "Account, skin & cape preview" },
  { to: "/settings", label: "Settings", icon: Settings, hint: "Appearance, Java, fixes, network" },
];

export function Sidebar() {
  const { data: status } = useNetworkStatus();
  const { data: lanWorlds } = useLanWorlds();
  const account = useActiveAccount();
  const hostCount = useSessionsStore((state) => Object.keys(state.hosts).length);
  const guestCount = useSessionsStore((state) => Object.keys(state.guests).length);
  // Selector must return a stable value: `.filter()` allocates a fresh array
  // on every call, which re-triggers the subscription and loops React into
  // "Maximum update depth exceeded" (#185) in production builds. A count is
  // all this badge needs.
  const activeJobCount = useJobsStore(
    (state) => state.jobs.filter((job) => !job.finished && !job.error).length,
  );
  const setActivityOpen = useUiStore((state) => state.setActivityOpen);

  /** Live counters on just two items: nearby LAN worlds and running jobs. */
  const badgeFor = (to: string): string | null => {
    if (to === "/servers") {
      const live = (status?.onlinePlayers ?? 0) + (lanWorlds?.length ?? 0);
      return live > 0 ? String(live) : null;
    }
    return null;
  };

  return (
    <nav className="no-drag flex shrink-0 items-center gap-1 overflow-x-auto border-b border-[var(--border)] bg-[oklch(0.14_0.012_70)] px-3">
      {NAV.map((item) => (
        <NavLink key={item.to} to={item.to} end={item.to === "/"}>
          {({ isActive }) => (
            <Hint label={item.hint} side="bottom">
              <div
                className={cn(
                  "flex items-center gap-2 border-b-2 px-3 py-3 text-sm font-semibold",
                  isActive
                    ? "border-[var(--primary)] text-[var(--foreground)]"
                    : "border-transparent text-[var(--muted-foreground)] hover:text-[var(--foreground)]",
                )}
              >
                <item.icon className={cn("size-4", isActive && "text-[var(--primary)]")} aria-hidden />
                <span>{item.label}</span>
                {badgeFor(item.to) ? (
                  <span className="text-[10px] tabular-nums text-[var(--primary)]">{badgeFor(item.to)}</span>
                ) : null}
              </div>
            </Hint>
          )}
        </NavLink>
      ))}
      <button
        type="button"
        onClick={() => setActivityOpen(true)}
        className="flex items-center gap-2 border-b-2 border-transparent px-3 py-3 text-sm font-semibold text-[var(--muted-foreground)] hover:text-[var(--foreground)]"
      >
        <Activity className="size-4" aria-hidden />
        Activity
        {activeJobCount > 0 ? (
          <span className="text-[10px] tabular-nums text-[var(--primary)]">{activeJobCount}</span>
        ) : null}
      </button>

      <div className="ml-auto flex items-center gap-2 py-2 pl-3">
        {hostCount > 0 || guestCount > 0 ? (
          <span className="flex items-center gap-1.5 text-xs">
            <StatusDot tone="success" pulse />
            {hostCount > 0 ? `${hostCount} hosting` : null}
            {guestCount > 0 ? `${guestCount} joined` : null}
          </span>
        ) : null}
        {BROWSER_MODE ? <Badge variant="warning">preview</Badge> : null}
        {account.data ? (
          <AccountMenu />
        ) : (
          <AccountMenu>
            <Button variant="outline" size="sm">
              <UserPlus className="size-4" /> Sign in
            </Button>
          </AccountMenu>
        )}
      </div>
    </nav>
  );
}
