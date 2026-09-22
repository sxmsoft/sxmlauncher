import { useEffect, useState } from "react";

import { useNavigate } from "react-router-dom";

import { ArrowUpCircle, Minus, Square, Wifi, WifiOff, X } from "lucide-react";

import { Badge, StatusDot } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Hint } from "@/components/ui/tooltip";
import { useNetworkStatus, useUpdateCheck } from "@/hooks/queries";
import { cn } from "@/lib/utils";
import { closeWindow, isWindowMaximized, minimizeWindow, toggleMaximizeWindow } from "@/lib/window";
import { useSessionsStore } from "@/stores/sessions";
import { useUiStore } from "@/stores/ui";

/**
 * Frameless titlebar.
 *
 * The whole bar is a drag region except the controls (`no-drag`), which is what
 * makes a `decorations: false` window feel native. It also carries the directory
 * status pill, so "am I reachable?" is answered on every page.
 */
export function TitleBar() {
  const [maximized, setMaximized] = useState(false);
  const { data: status } = useNetworkStatus();
  const { data: update } = useUpdateCheck({ auto: true });
  const navigate = useNavigate();

  const guestCount = useSessionsStore((state) => Object.keys(state.guests).length);
  const hostCount = useSessionsStore((state) => Object.keys(state.hosts).length);

  useEffect(() => {
    void isWindowMaximized().then(setMaximized);
  }, []);

  const connected = status?.directoryConnected ?? false;

  return (
    <div className="drag-region flex h-11 shrink-0 items-center justify-between gap-3 border-b border-white/6 px-3">
      <div className="flex items-center gap-2 pl-1">
        <img
          src="/icon.png"
          alt="SXMLauncher"
          className="size-6 rounded-md object-contain"
          draggable={false}
        />
        <span className="text-sm font-semibold tracking-tight">SXMLauncher</span>
        <span className="text-muted-foreground/60 hidden text-xs sm:inline">
          p2p worlds · modpacks
        </span>
      </div>

      <div className="no-drag flex items-center gap-2">
        {update?.updateAvailable ? (
          <Hint label={`Version ${update.version} is available — click to install it from Settings → Updates`}>
            <button
              type="button"
              onClick={() => {
                useUiStore.getState().openSettings("updates");
                void navigate("/settings");
              }}
              className="focus-visible:ring-ring rounded-full focus-visible:outline-none"
            >
              <Badge variant="primary">
                <ArrowUpCircle className="size-3" />
                update
              </Badge>
            </button>
          </Hint>
        ) : null}
        {hostCount > 0 ? (
          <Badge variant="success">
            <StatusDot tone="success" pulse />
            hosting {hostCount}
          </Badge>
        ) : null}
        {guestCount > 0 ? <Badge variant="primary">in {guestCount} session</Badge> : null}

        <Hint
          label={
            connected
              ? `Server directory: ${status?.directoryUrl ?? ""}`
              : (status?.message ?? "Server directory is unreachable — check the Redis URL in Settings")
          }
        >
          <Badge variant={connected ? "outline" : "warning"}>
            {connected ? <Wifi className="size-3" /> : <WifiOff className="size-3" />}
            {connected ? `${status?.onlinePlayers ?? 0} online` : "offline"}
          </Badge>
        </Hint>

        <div className="flex items-center gap-0.5 pl-1">
          <Button variant="ghost" size="icon-sm" onClick={() => void minimizeWindow()}>
            <Minus className="size-3.5" />
            <span className="sr-only">Minimize</span>
          </Button>
          <Button
            variant="ghost"
            size="icon-sm"
            onClick={() => {
              void toggleMaximizeWindow();
              void isWindowMaximized().then(setMaximized);
            }}
          >
            <Square className={cn("size-3", maximized && "opacity-60")} />
            <span className="sr-only">Maximize</span>
          </Button>
          <Button
            variant="ghost"
            size="icon-sm"
            className="hover:bg-[var(--destructive)]/80 hover:text-white"
            onClick={() => void closeWindow()}
          >
            <X className="size-3.5" />
            <span className="sr-only">Close</span>
          </Button>
        </div>
      </div>
    </div>
  );
}
