import { Outlet } from "react-router-dom";

import { ActivityPanel } from "@/components/jobs/activity-panel";
import { Sidebar } from "@/components/layout/sidebar";
import { StatusStrip } from "@/components/layout/status-strip";
import { TitleBar } from "@/components/layout/title-bar";
import { Toaster } from "@/components/ui/toaster";
import { TooltipProvider } from "@/components/ui/tooltip";
import { useAutoUpdateCheck } from "@/hooks/queries";
import { useBackendEvents } from "@/hooks/use-backend-events";
import { useNoDropNavigation } from "@/hooks/use-no-drop-navigation";
import { useWallpaper } from "@/hooks/use-wallpaper";

/**
 * The application frame: frameless titlebar, side navigation, scrolling content
 * and the live status strip.
 *
 * This is also the single place where backend events are wired into the stores —
 * exactly once, so a route change never tears the subscriptions down.
 */
export function AppShell() {
  useBackendEvents();
  useWallpaper();
  useNoDropNavigation();
  // Startup update check: at most once a day, silent unless it finds a
  // newer release — then one toast plus the titlebar badge take over.
  useAutoUpdateCheck();

  return (
    <TooltipProvider delayDuration={200} skipDelayDuration={300}>
      <div className="app-aurora relative flex h-full flex-col overflow-hidden">
        <div className="app-wallpaper" aria-hidden />
        <div className="relative z-10 flex min-h-0 flex-1 flex-col">
          <TitleBar />
          <Sidebar />
          <main className="min-h-0 flex-1 overflow-y-auto px-8 py-6">
            <Outlet />
          </main>
          <StatusStrip />
        </div>
        <ActivityPanel />
        <Toaster />
      </div>
    </TooltipProvider>
  );
}
