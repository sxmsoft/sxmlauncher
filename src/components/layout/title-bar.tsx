import { useEffect, useState } from "react";

import { useNavigate } from "react-router-dom";

import { Activity, ArrowUpCircle, Minus, Search, Square, Wifi, WifiOff, X } from "lucide-react";
import { useTranslation } from "react-i18next";

import { BrandMark } from "@/components/brand/logo";
import { AccountMenu } from "@/components/account/account-menu";
import { AccountAvatar } from "@/components/account/account-avatar";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Hint } from "@/components/ui/tooltip";
import { useActiveAccount, useNetworkStatus, useUpdateCheck } from "@/hooks/queries";
import { cn } from "@/lib/utils";
import { closeWindow, isWindowMaximized, minimizeWindow, toggleMaximizeWindow } from "@/lib/window";
import { BROWSER_MODE } from "@/services";
import { useJobsStore } from "@/stores/jobs";
import { useUiStore } from "@/stores/ui";

/**
 * Top bar: wordmark, library search, account, and the frameless window controls.
 * The bar itself is the drag region; interactive bits opt out.
 */
export function TitleBar() {
  const [maximized, setMaximized] = useState(false);
  const { data: status } = useNetworkStatus();
  const { data: update } = useUpdateCheck({ auto: true });
  const account = useActiveAccount();
  const navigate = useNavigate();
  const query = useUiStore((state) => state.chromeQuery);
  const setQuery = useUiStore((state) => state.setChromeQuery);
  const setActivityOpen = useUiStore((state) => state.setActivityOpen);
  const activeJobCount = useJobsStore(
    (state) => state.jobs.filter((job) => !job.finished && !job.error).length,
  );

  useEffect(() => {
    void isWindowMaximized().then(setMaximized);
  }, []);

  const { t } = useTranslation();
  const connected = status?.directoryConnected ?? false;

  return (
    <header
      className="drag-region z-20 flex shrink-0 items-center gap-4 border-b border-[var(--border)] px-5"
      style={{
        height: "var(--topbar-h)",
        background: "color-mix(in srgb, var(--surface-1) 80%, transparent)",
        backdropFilter: "blur(var(--blur-panel))",
      }}
    >
      <div className="flex items-center gap-2 text-[15px] font-bold tracking-[0.08em] whitespace-nowrap">
        <BrandMark className="size-5" />
        SXM<span className="font-semibold text-[var(--accent)]">LAUNCHER</span>
      </div>

      <label className="glass-pill no-drag ml-2 flex h-9 max-w-[360px] flex-1 items-center gap-2.5 px-3.5 text-[var(--text-muted)] focus-within:shadow-[0_0_0_3px_var(--accent-dim)]">
        <Search className="size-4 shrink-0" strokeWidth={1.5} aria-hidden />
        <input
          type="search"
          value={query}
          onChange={(event) => setQuery(event.target.value)}
          placeholder={t("chrome.search")}
          aria-label={t("chrome.searchLabel")}
          className="min-w-0 flex-1 bg-transparent text-[13px] text-[var(--text)] outline-none placeholder:text-[var(--text-faint)]"
        />
        <kbd className="hidden rounded border border-[var(--border)] bg-[var(--surface-3)] px-1.5 py-0.5 font-mono text-[10px] text-[var(--text-faint)] sm:inline">
          /
        </kbd>
      </label>

      <div className="no-drag ml-auto flex items-center gap-2">
        {BROWSER_MODE ? <Badge variant="warning">{t("chrome.preview")}</Badge> : null}
        {update?.updateAvailable ? (
          <Hint label={`Version ${update.version} is available`}>
            <button
              type="button"
              onClick={() => {
                useUiStore.getState().openSettings("updates");
                void navigate("/settings");
              }}
              className="rounded-full"
            >
              <Badge variant="primary">
                <ArrowUpCircle className="size-3" />
                {t("chrome.update")}
              </Badge>
            </button>
          </Hint>
        ) : null}

        <Hint
          label={
            connected
              ? `Directory: ${status?.directoryUrl ?? ""}`
              : (status?.message ?? "Directory unreachable — LAN and join codes still work")
          }
        >
          <Badge variant={connected ? "outline" : "warning"}>
            {connected ? <Wifi className="size-3" /> : <WifiOff className="size-3" />}
            {connected ? t("chrome.online", { count: status?.onlinePlayers ?? 0 }) : t("chrome.offline")}
          </Badge>
        </Hint>

        <button
          type="button"
          title={t("chrome.activity")}
          onClick={() => setActivityOpen(true)}
          className="relative grid size-9 place-items-center rounded-[10px] text-[var(--text-muted)] hover:border hover:border-[var(--border)] hover:bg-[var(--accent-dim)] hover:text-[var(--text)]"
        >
          <Activity className="size-[18px]" strokeWidth={1.75} />
          {activeJobCount > 0 ? (
            <span className="absolute -top-1 -right-1 rounded-full bg-[var(--accent)] px-1 font-mono text-[9px] text-white">
              {activeJobCount}
            </span>
          ) : null}
          <span className="sr-only">{t("chrome.activity")}</span>
        </button>

        <AccountMenu>
          <button
            type="button"
            className="grid size-9 place-items-center overflow-hidden rounded-[10px] border border-[var(--border-strong)] bg-[var(--surface-3)] shadow-[0_0_0_2px_var(--accent-dim)]"
            aria-label={t("chrome.profile")}
          >
            {account.data ? (
              <AccountAvatar account={account.data} size={36} />
            ) : (
              <span className="text-[11px] font-semibold text-[var(--accent-soft)]">+</span>
            )}
          </button>
        </AccountMenu>

        <div className="ml-1 flex items-center gap-0.5">
          <Button variant="ghost" size="icon-sm" onClick={() => void minimizeWindow()}>
            <Minus className="size-3.5" />
            <span className="sr-only">{t("chrome.minimize")}</span>
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
            <span className="sr-only">{t("chrome.maximize")}</span>
          </Button>
          <Button
            variant="ghost"
            size="icon-sm"
            className="hover:bg-[var(--destructive)]/80 hover:text-white"
            onClick={() => void closeWindow()}
          >
            <X className="size-3.5" />
            <span className="sr-only">{t("chrome.close")}</span>
          </Button>
        </div>
      </div>
    </header>
  );
}
