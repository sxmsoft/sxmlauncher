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
    <aside className="flex w-56 shrink-0 flex-col gap-3 border-r border-white/6 py-3">
      <nav className="flex flex-col gap-1 px-2">
        {NAV.map((item) => (
          <NavLink key={item.to} to={item.to} end={item.to === "/"} className="group">
            {({ isActive }) => (
              <Hint label={item.hint} side="right">
                <div
                  className={cn(
                    "flex w-full items-center gap-3 rounded-lg px-3 py-2 text-sm font-medium transition-all duration-150",
                    isActive
                      ? "bg-white/10 text-[var(--foreground)] shadow-[inset_0_1px_0_rgba(255,255,255,0.06)]"
                      : "text-[var(--muted-foreground)] hover:translate-x-0.5 hover:bg-white/6 hover:text-[var(--foreground)]",
                  )}
                >
                  <item.icon
                    className={cn("size-4", isActive && "text-[var(--primary)]")}
                    aria-hidden
                  />
                  <span className="flex-1">{item.label}</span>
                  {badgeFor(item.to) ? (
                    <span className="text-muted-foreground text-[10px] tabular-nums">
                      {badgeFor(item.to)}
                    </span>
                  ) : null}
                </div>
              </Hint>
            )}
          </NavLink>
        ))}
        <button
          onClick={() => setActivityOpen(true)}
          className="flex w-full items-center gap-3 rounded-lg px-3 py-2 text-sm font-medium text-[var(--muted-foreground)] transition-all duration-150 hover:translate-x-0.5 hover:bg-white/6 hover:text-[var(--foreground)]"
        >
          <Activity className="size-4" aria-hidden />
          <span className="flex-1 text-left">Activity</span>
          {activeJobCount > 0 ? (
            <span className="text-[var(--primary)] text-[10px] tabular-nums">
              {activeJobCount}
            </span>
          ) : null}
        </button>
      </nav>

      <div className="mt-auto flex flex-col gap-2 px-3">
        {(hostCount > 0 || guestCount > 0) && (
          <div className="flex flex-col gap-1 rounded-lg border border-white/8 bg-black/20 p-2.5 text-xs">
            <span className="text-muted-foreground flex items-center gap-1.5 font-medium">
              <StatusDot tone="success" pulse />
              live sessions
            </span>
            {hostCount > 0 ? <span>hosting {hostCount} world{hostCount === 1 ? "" : "s"}</span> : null}
            {guestCount > 0 ? <span>joined {guestCount}</span> : null}
          </div>
        )}

        {BROWSER_MODE ? (
          <Badge variant="warning" className="justify-center">
            browser preview · mock data
          </Badge>
        ) : null}

        {account.data ? (
          <AccountMenu />
        ) : (
          <AccountMenu>
            <Button variant="outline" size="sm" className="w-full justify-start">
              <UserPlus className="size-4" />
              Add an account
            </Button>
          </AccountMenu>
        )}
      </div>
    </aside>
  );
}
