import { Suspense, lazy, type ReactNode } from "react";

import { QueryClientProvider } from "@tanstack/react-query";
import { HashRouter, Navigate, Route, Routes, useLocation } from "react-router-dom";

import { ErrorBoundary } from "@/components/common/error-boundary";
import { AppShell } from "@/components/layout/app-shell";
import { Spinner } from "@/components/ui/feedback";
import { queryClient } from "@/lib/query-client";

/**
 * Pages are loaded on demand so the launcher window opens on the dashboard
 * without paying for the settings screen, the mod browser and the server
 * browser up front. Each import becomes its own chunk.
 *
 * The pages are named exports, so every `lazy()` maps the module to a
 * `default` export rather than us adding a redundant default to each file.
 */
const DashboardPage = lazy(async () => ({
  default: (await import("@/pages/dashboard")).DashboardPage,
}));
const InstanceDetailPage = lazy(async () => ({
  default: (await import("@/pages/instance-detail")).InstanceDetailPage,
}));
const ModpacksPage = lazy(async () => ({
  default: (await import("@/pages/modpacks")).ModpacksPage,
}));
const CustomPacksPage = lazy(async () => ({
  default: (await import("@/pages/customPacks")).CustomPacksPage,
}));
const ServersPage = lazy(async () => ({
  default: (await import("@/pages/servers")).ServersPage,
}));
const SkinPage = lazy(async () => ({
  default: (await import("@/pages/skin")).SkinPage,
}));
const SettingsPage = lazy(async () => ({
  default: (await import("@/pages/settings")).SettingsPage,
}));
const LibraryPage = lazy(async () => ({
  default: (await import("@/pages/library")).LibraryPage,
}));
const ActivityPage = lazy(async () => ({
  default: (await import("@/pages/activity")).ActivityPage,
}));

/**
 * Suspense boundary for a single route.
 *
 * It sits *inside* [`AppShell`] on purpose: the sidebar, title bar and status
 * strip must stay mounted while a page chunk is still loading, otherwise the
 * whole window would flash a spinner on every navigation. The `page-enter`
 * animation replays per route so navigation always feels alive.
 */
function Page({ children }: { children: ReactNode }) {
  const location = useLocation();
  return (
    <div key={location.pathname} className="page-enter">
      <Suspense
        fallback={
          <div className="flex h-full min-h-[24rem] items-center justify-center">
            <Spinner />
          </div>
        }
      >
        {children}
      </Suspense>
    </div>
  );
}

/**
 * Root component.
 *
 * Hash routing on purpose: the production build is served from Tauri's custom
 * protocol, where path-based routes would need server-side rewrites that a
 * desktop webview does not have.
 */
export function App() {
  return (
    <ErrorBoundary>
      <QueryClientProvider client={queryClient}>
        <HashRouter>
          <Routes>
            <Route element={<AppShell />}>
              <Route
                index
                element={
                  <Page>
                    <DashboardPage />
                  </Page>
                }
              />
              <Route
                path="library"
                element={
                  <Page>
                    <LibraryPage />
                  </Page>
                }
              />
              <Route
                path="activity"
                element={
                  <Page>
                    <ActivityPage />
                  </Page>
                }
              />
              <Route
                path="instances/:id"
                element={
                  <Page>
                    <InstanceDetailPage />
                  </Page>
                }
              />
              <Route
                path="modpacks"
                element={
                  <Page>
                    <ModpacksPage />
                  </Page>
                }
              />
              <Route
                path="custom-packs"
                element={
                  <Page>
                    <CustomPacksPage />
                  </Page>
                }
              />
              <Route
                path="servers"
                element={
                  <Page>
                    <ServersPage />
                  </Page>
                }
              />
              <Route
                path="skin"
                element={
                  <Page>
                    <SkinPage />
                  </Page>
                }
              />
              <Route
                path="settings"
                element={
                  <Page>
                    <SettingsPage />
                  </Page>
                }
              />
              <Route path="*" element={<Navigate to="/" replace />} />
            </Route>
          </Routes>
        </HashRouter>
      </QueryClientProvider>
    </ErrorBoundary>
  );
}
