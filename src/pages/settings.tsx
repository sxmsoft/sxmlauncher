import { useEffect, useRef, useState } from "react";

import {
  Bug,
  Cog,
  Cpu,
  FolderOpen,
  Globe,
  HardDrive,
  KeyRound,
  Palette,
  RefreshCw,
  Save,
  ScrollText,
  Trash,
  Users,
  Zap,
} from "lucide-react";

import { AccountMenu } from "@/components/account/account-menu";
import { PageHeader } from "@/components/common/page-header";
import { AppearanceSection } from "@/components/settings/appearance-section";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Progress } from "@/components/ui/progress";
import { ConfirmDialog } from "@/components/ui/dialog";
import { Separator, Skeleton, Stat } from "@/components/ui/feedback";
import { Field, Input, SettingRow } from "@/components/ui/input";
import { NumberInput, Switch } from "@/components/ui/switch";
import {
  useAppInfo,
  useAppPaths,
  useCacheStats,
  useClearCache,
  useInstallJava,
  useJavaRuntimes,
  useNatProbe,
  useSaveSettings,
  useSessionHistory,
  useSettings,
  useTestRedis,
  useUpdateCheck,
  useUpdateInstaller,
  useVaultBackend,
} from "@/hooks/queries";
import { applyAppearance } from "@/lib/appearance";
import { formatBytes, formatDuration, formatRelative } from "@/lib/utils";
import { BROWSER_MODE, systemService } from "@/services";
import { revealPath } from "@/lib/window";
import { useUiStore, toast } from "@/stores/ui";
import type { AppSettings, SettingsSection } from "@/types/system";

function withAppearance(base: AppSettings, appearance: AppSettings): AppSettings {
  return {
    ...base,
    theme: appearance.theme,
    accent: appearance.accent,
    uiAccent: appearance.uiAccent,
    uiBackgroundKind: appearance.uiBackgroundKind,
    uiBackgroundPath: appearance.uiBackgroundPath,
    uiBackgroundOpacity: appearance.uiBackgroundOpacity,
    uiBackgroundBlur: appearance.uiBackgroundBlur,
    uiAnimations: appearance.uiAnimations,
    uiCompact: appearance.uiCompact,
    reduceMotion: appearance.reduceMotion,
  };
}

const SECTIONS: Array<{ id: SettingsSection; label: string; icon: typeof Cog }> = [
  { id: "general", label: "General", icon: Cog },
  { id: "appearance", label: "Appearance", icon: Palette },
  { id: "downloads", label: "Downloads", icon: HardDrive },
  { id: "defaults", label: "Game defaults", icon: Cpu },
  { id: "network", label: "Network & hosting", icon: Globe },
  { id: "accounts", label: "Accounts & vault", icon: KeyRound },
  { id: "storage", label: "Storage", icon: FolderOpen },
  { id: "updates", label: "Updates", icon: RefreshCw },
  { id: "diagnostics", label: "Diagnostics", icon: Bug },
];

/**
 * Settings.
 *
 * The whole settings object is edited as one draft and written back in a single
 * `settings_update`. That matters because the backend rebuilds its managers from
 * the new settings (download queue, CurseForge client, directory connection) —
 * sending partial patches would mean rebuilding twice and could leave a manager
 * holding half of a configuration.
 */
export function SettingsPage() {
  const { data: settings, isLoading } = useSettings();
  const save = useSaveSettings();
  const section = useUiStore((state) => state.settingsSection);
  const setSection = useUiStore((state) => state.setSettingsSection);

  const [draft, setDraft] = useState<AppSettings | null>(null);
  const draftRef = useRef<AppSettings | null>(null);
  const saveTimer = useRef<number | null>(null);

  useEffect(() => {
    if (!settings) return;
    setDraft((current) => {
      if (!current) return settings;
      return withAppearance(current, settings);
    });
  }, [settings]);

  draftRef.current = draft;

  const dirty = draft != null && settings != null && JSON.stringify(draft) !== JSON.stringify(settings);
  const patch = (next: Partial<AppSettings>) =>
    setDraft((current) => (current ? { ...current, ...next } : current));

  /**
   * Appearance paints immediately and persists on its own, so a slider does not
   * wait on the global Save button. Other dirty fields stay in the draft.
   */
  const commitAppearance = (next: Partial<AppSettings>) => {
    const current = draftRef.current;
    if (!current || !settings) return;
    const merged = { ...current, ...next };
    setDraft(merged);
    applyAppearance(merged);
    if (saveTimer.current != null) window.clearTimeout(saveTimer.current);
    saveTimer.current = window.setTimeout(() => {
      save.mutate(withAppearance(settings, merged));
    }, 280);
  };

  if (isLoading || !draft) {
    return (
      <div className="flex flex-col gap-4">
        <Skeleton className="h-8 w-48" />
        <Skeleton className="h-96 w-full" />
      </div>
    );
  }

  return (
    <div className="flex flex-col gap-5">
      <PageHeader
        title="Settings"
        description="Everything is stored locally. Only the server directory URL touches the network."
        actions={
          <>
            {dirty ? <Badge variant="warning">unsaved changes</Badge> : null}
            <Button
              size="sm"
              disabled={!dirty}
              loading={save.isPending}
              onClick={() => draft && save.mutate(draft)}
            >
              <Save className="size-4" /> Save settings
            </Button>
          </>
        }
      />

      <div className="grid gap-5 lg:grid-cols-[220px_minmax(0,1fr)]">
        <nav className="flex flex-col gap-1">
          {SECTIONS.map((entry) => (
            <button
              key={entry.id}
              type="button"
              onClick={() => setSection(entry.id)}
              className={
                "flex items-center gap-3 rounded-lg px-3 py-2 text-left text-sm font-medium transition-colors " +
                (section === entry.id
                  ? "bg-white/10 text-[var(--foreground)]"
                  : "text-muted-foreground hover:bg-white/6 hover:text-[var(--foreground)]")
              }
            >
              <entry.icon className="size-4" />
              {entry.label}
            </button>
          ))}
        </nav>

        <div className="flex flex-col gap-5">
          {section === "general" ? <GeneralSection draft={draft} patch={patch} /> : null}
          {section === "appearance" ? (
            <AppearanceSection draft={draft} onChange={commitAppearance} />
          ) : null}
          {section === "downloads" ? <DownloadsSection draft={draft} patch={patch} /> : null}
          {section === "defaults" ? <DefaultsSection draft={draft} patch={patch} /> : null}
          {section === "network" ? <NetworkSection draft={draft} patch={patch} /> : null}
          {section === "accounts" ? <AccountsSection /> : null}
          {section === "storage" ? <StorageSection /> : null}
          {section === "updates" ? <UpdatesSection /> : null}
          {section === "diagnostics" ? <DiagnosticsSection /> : null}
        </div>
      </div>
    </div>
  );
}

type SectionProps = { draft: AppSettings; patch: (next: Partial<AppSettings>) => void };

function GeneralSection({ draft, patch }: SectionProps) {
  const info = useAppInfo();

  return (
    <>
      <Card>
        <CardHeader>
          <CardTitle>Window & tray</CardTitle>
        </CardHeader>
        <CardContent>
          <SettingRow
            title="Minimise to the tray while a game runs"
            description="Keeps the launcher out of the way after launch."
            control={
              <Switch
                checked={draft.minimizeToTrayOnLaunch}
                onCheckedChange={(value) => patch({ minimizeToTrayOnLaunch: value })}
              />
            }
          />
          <SettingRow
            title="Closing the window keeps the launcher running"
            description="Hosted worlds stay online until you quit from the tray."
            control={<Switch checked={draft.closeToTray} onCheckedChange={(value) => patch({ closeToTray: value })} />}
          />
        </CardContent>
      </Card>

      <Card>
        <CardHeader>
          <CardTitle>CurseForge</CardTitle>
        </CardHeader>
        <CardContent className="flex flex-col gap-3">
          <Field
            label="API key"
            hint="Stored in the OS credential vault, never in the settings file. Modrinth needs no key."
          >
            <Input
              type="password"
              value={draft.curseforgeApiKey ?? ""}
              onChange={(event) => patch({ curseforgeApiKey: event.target.value || null })}
              placeholder={info.data?.curseforgeConfigured ? "•••••••• (configured)" : "paste your key"}
            />
          </Field>
          <p className="text-muted-foreground text-xs leading-relaxed">
            CurseForge disabled third-party downloads without a key. Without one, modpack
            installs that come from CurseForge will fail — Modrinth keeps working.
          </p>
        </CardContent>
      </Card>

      <Card>
        <CardHeader>
          <CardTitle>Anonymous usage</CardTitle>
        </CardHeader>
        <CardContent>
          <SettingRow
            title="Send anonymous diagnostics"
            description="Session success rates only. Never usernames, world names or IP addresses."
            control={
              <Switch checked={draft.analyticsEnabled} onCheckedChange={(value) => patch({ analyticsEnabled: value })} />
            }
          />
        </CardContent>
      </Card>
    </>
  );
}

function DownloadsSection({ draft, patch }: SectionProps) {
  return (
    <Card>
      <CardHeader>
        <CardTitle>Downloads</CardTitle>
      </CardHeader>
      <CardContent>
        <div className="py-3">
          <Field label="Parallel downloads" hint="Higher is faster on good connections, riskier on flaky ones.">
            <NumberInput
              value={draft.maxConcurrentDownloads}
              min={1}
              max={64}
              onValueChange={(value) => patch({ maxConcurrentDownloads: value })}
              suffix="files"
            />
          </Field>
        </div>
        <SettingRow
          title="Re-download on hash mismatch"
          description="Fetches a file once more instead of failing the job. Hashes are always verified."
          control={
            <Switch
              checked={draft.reDownloadOnHashMismatch}
              onCheckedChange={(value) => patch({ reDownloadOnHashMismatch: value })}
            />
          }
        />
        <SettingRow
          title="Segmented downloads"
          description="Uses HTTP range requests for large files such as the client jar and asset batches."
          control={
            <Switch
              checked={draft.enableRangeRequests}
              onCheckedChange={(value) => patch({ enableRangeRequests: value })}
            />
          }
        />
        <SettingRow
          title="Keep the download cache"
          description="Content-addressed by hash, so two instances share one copy of a mod."
          control={
            <Switch
              checked={draft.keepDownloadCache}
              onCheckedChange={(value) => patch({ keepDownloadCache: value })}
            />
          }
        />
      </CardContent>
    </Card>
  );
}

function DefaultsSection({ draft, patch }: SectionProps) {
  const runtimes = useJavaRuntimes();
  const installJava = useInstallJava();

  return (
    <>
      <Card>
        <CardHeader>
          <CardTitle>New instances</CardTitle>
        </CardHeader>
        <CardContent className="flex flex-col gap-4">
          <div className="grid gap-4 sm:grid-cols-2">
            <Field label="Minimum memory">
              <NumberInput
                value={draft.defaultMemory.minMb}
                min={512}
                max={32768}
                step={512}
                suffix="MB"
                onValueChange={(value) => patch({ defaultMemory: { ...draft.defaultMemory, minMb: value } })}
              />
            </Field>
            <Field label="Maximum memory">
              <NumberInput
                value={draft.defaultMemory.maxMb}
                min={512}
                max={32768}
                step={512}
                suffix="MB"
                onValueChange={(value) => patch({ defaultMemory: { ...draft.defaultMemory, maxMb: value } })}
              />
            </Field>
          </div>
          <SettingRow
            title="Provision Java automatically"
            description="Downloads the JDK a version needs, matching Temurin builds per platform."
            control={
              <Switch checked={draft.autoProvisionJava} onCheckedChange={(value) => patch({ autoProvisionJava: value })} />
            }
          />
        </CardContent>
      </Card>

      <Card>
        <CardHeader className="flex-row items-center justify-between">
          <CardTitle>Java runtimes</CardTitle>
          <Button
            size="sm"
            variant="outline"
            onClick={() => installJava.mutate(21)}
            loading={installJava.isPending}
          >
            <Zap className="size-4" /> Provision Java 21
          </Button>
        </CardHeader>
        <CardContent className="flex flex-col gap-2">
          {(runtimes.data ?? []).map((runtime) => (
            <div
              key={runtime.path}
              className="flex flex-wrap items-center gap-3 rounded-lg border border-white/8 bg-black/20 px-3 py-2"
            >
              <Badge variant={runtime.isManaged ? "primary" : "outline"}>Java {runtime.major}</Badge>
              <span className="text-sm">{runtime.vendor}</span>
              <span className="text-muted-foreground text-xs">{runtime.version}</span>
              <span className="text-muted-foreground ml-auto max-w-72 truncate text-[11px]">
                {runtime.path}
              </span>
            </div>
          ))}
          {runtimes.data?.length === 0 ? (
            <p className="text-muted-foreground text-sm">
              No Java found. Provision a runtime here, or install one with your package manager.
            </p>
          ) : null}
        </CardContent>
      </Card>
    </>
  );
}

function NetworkSection({ draft, patch }: SectionProps) {
  const testRedis = useTestRedis();

  return (
    <>
      <Card>
        <CardHeader>
          <CardTitle>Server directory</CardTitle>
        </CardHeader>
        <CardContent className="flex flex-col gap-4">
          <Field
            label="Directory broker"
            hint="The embedded directory meets friends on a public MQTT broker — no server to run. Leave empty to use a self-hosted Redis instead."
          >
            <div className="flex gap-2">
              <Input
                value={draft.mqttBroker}
                onChange={(event) => patch({ mqttBroker: event.target.value })}
                placeholder="broker.emqx.io"
              />
              <Input
                className="w-28"
                value={String(draft.mqttPort)}
                onChange={(event) =>
                  patch({ mqttPort: Number(event.target.value) || 8883 })
                }
              />
              <Button
                variant="outline"
                onClick={() =>
                  testRedis.mutate(
                    draft.mqttBroker.trim()
                      ? `mqtt://${draft.mqttBroker.trim()}:${draft.mqttPort}`
                      : draft.redisUrl,
                    {
                      onSuccess: (probe) =>
                        probe.ok
                          ? toast.success("Directory reachable", `${probe.onlinePlayers} players online`)
                          : toast.error(probe.message ?? "Connection failed", "Directory unreachable"),
                    },
                  )
                }
                loading={testRedis.isPending}
              >
                <RefreshCw className="size-4" /> Test
              </Button>
            </div>
          </Field>

          <Field
            label="Redis URL (advanced)"
            hint="Only used when the broker field above is empty. Self-hosting the old Redis directory is still supported."
          >
            <div className="flex gap-2">
              <Input
                value={draft.redisUrl}
                onChange={(event) => patch({ redisUrl: event.target.value })}
                placeholder="redis://127.0.0.1:6379/0"
              />
              <Button
                variant="outline"
                onClick={() =>
                  testRedis.mutate(draft.redisUrl, {
                    onSuccess: (probe) =>
                      probe.ok
                        ? toast.success("Directory reachable", `${probe.onlinePlayers} players online`)
                        : toast.error(probe.message ?? "Connection failed", "Directory unreachable"),
                  })
                }
                loading={testRedis.isPending}
              >
                <RefreshCw className="size-4" /> Test
              </Button>
            </div>
          </Field>

          <Field label="Relay URL" hint="Used when a peer's NAT cannot be punched through.">
            <Input
              value={draft.relayUrl ?? ""}
              onChange={(event) => patch({ relayUrl: event.target.value || null })}
              placeholder="wss://relay.example.dev"
            />
          </Field>

          <Field label="STUN servers" hint="Comma separated. Used to classify your NAT and find your public endpoint.">
            <Input
              value={draft.stunServers.join(", ")}
              onChange={(event) =>
                patch({
                  stunServers: event.target.value
                    .split(",")
                    .map((entry) => entry.trim())
                    .filter(Boolean),
                })
              }
            />
          </Field>
        </CardContent>
      </Card>

      <Card>
        <CardHeader>
          <CardTitle>Hosting defaults</CardTitle>
        </CardHeader>
        <CardContent>
          <div className="py-3">
            <Field label="Maximum players per hosted world">
              <NumberInput
                value={draft.maxHostedPlayers}
                min={1}
                max={128}
                onValueChange={(value) => patch({ maxHostedPlayers: value })}
                suffix="players"
              />
            </Field>
          </div>
          <SettingRow
            title="Publish hosted worlds by default"
            description="Off means share-code only: friends must be given the code."
            control={<Switch checked={draft.shareByDefault} onCheckedChange={(value) => patch({ shareByDefault: value })} />}
          />
          <SettingRow
            title="Advertise local network endpoints"
            description="On by default. Loopback and private LAN addresses are always shared so a second launcher on this PC can join when the relay hostname does not resolve. This switch also shares any other interface address."
            control={
              <Switch checked={draft.exposeLanEndpoints} onCheckedChange={(value) => patch({ exposeLanEndpoints: value })} />
            }
          />
          <SettingRow
            title="Default host password"
            description="Applied to new sessions; can be overridden per world."
            control={
              <Switch
                checked={draft.hostPassword != null}
                onCheckedChange={(value) => patch({ hostPassword: value ? "" : null })}
              />
            }
          />
          {draft.hostPassword != null ? (
            <div className="pt-3">
              <Field label="Host password">
                <Input
                  type="password"
                  value={draft.hostPassword}
                  onChange={(event) => patch({ hostPassword: event.target.value })}
                />
              </Field>
            </div>
          ) : null}
        </CardContent>
      </Card>
    </>
  );
}

function AccountsSection() {
  const vault = useVaultBackend();

  return (
    <Card>
      <CardHeader>
        <CardTitle>Accounts</CardTitle>
      </CardHeader>
      <CardContent className="flex flex-col gap-4">
        <div className="flex items-center gap-3">
          <AccountMenu />
          <span className="text-muted-foreground text-xs">
            Tokens are written to the credential vault, never to disk in this app's data
            directory.
          </span>
        </div>
        <Separator />
        <div className="grid gap-4 sm:grid-cols-2">
          <Stat label="Vault backend" value={vault.data ?? "unknown"} />
          <Stat
            label="Providers"
            value="Microsoft · Ely.by · Offline"
          />
        </div>
        <p className="text-muted-foreground text-xs leading-relaxed">
          Signing out removes the refresh token from the vault immediately; the account entry
          is kept so you can sign back in with one click. Access tokens live only for the
          lifetime of a launch and are never written to disk.
        </p>
      </CardContent>
    </Card>
  );
}

function StorageSection() {
  const paths = useAppPaths();
  const stats = useCacheStats();
  const clear = useClearCache();
  const [confirm, setConfirm] = useState(false);

  return (
    <>
      <Card>
        <CardHeader className="flex-row items-center justify-between">
          <CardTitle>Cache</CardTitle>
          <Button variant="outline" size="sm" onClick={() => setConfirm(true)}>
            <Trash className="size-4" /> Clear cache
          </Button>
        </CardHeader>
        <CardContent className="grid gap-4 sm:grid-cols-3">
          <Stat label="Downloads" value={formatBytes(stats.data?.downloadBytes ?? 0)} />
          <Stat label="Metadata rows" value={stats.data?.metadataRows ?? 0} />
          <Stat label="Instances" value={formatBytes(stats.data?.instanceBytes ?? 0)} />
        </CardContent>
      </Card>

      <Card>
        <CardHeader>
          <CardTitle>Folders</CardTitle>
        </CardHeader>
        <CardContent className="flex flex-col gap-2">
          {Object.entries(paths.data ?? {}).map(([label, path]) => (
            <div
              key={label}
              className="flex items-center gap-3 rounded-lg border border-white/8 bg-black/20 px-3 py-2"
            >
              <span className="text-muted-foreground w-24 text-[10px] font-medium tracking-widest uppercase">
                {label}
              </span>
              <span className="flex-1 truncate text-xs">{path}</span>
              <Button variant="ghost" size="icon-sm" onClick={() => void revealPath(path)}>
                <FolderOpen className="size-3.5" />
                <span className="sr-only">Reveal {label}</span>
              </Button>
            </div>
          ))}
        </CardContent>
      </Card>

      <ConfirmDialog
        open={confirm}
        onOpenChange={setConfirm}
        title="Clear the download cache?"
        description="Verified files are deleted and will be downloaded again when needed. Instances themselves are not touched."
        confirmLabel="Clear cache"
        busy={clear.isPending}
        onConfirm={() =>
          clear.mutate({ downloads: true, metadata: true }, { onSuccess: () => setConfirm(false) })
        }
      />
    </>
  );
}

/**
 * Updates: feed check, signature-verified download, restart-to-apply.
 *
 * In the browser preview the updater commands do not exist, so the section
 * renders the mock "up to date" state instead of offering broken buttons.
 */
function UpdatesSection() {
  const info = useAppInfo();
  const check = useUpdateCheck();
  const installer = useUpdateInstaller();

  const available = check.data?.updateAvailable ?? false;
  const total = installer.progress?.totalBytes ?? null;
  const done = installer.progress?.downloadedBytes ?? 0;
  const percent = total && total > 0 ? Math.min(100, (done / total) * 100) : null;

  return (
    <>
      <Card>
        <CardHeader className="flex-row items-center justify-between">
          <CardTitle>Launcher updates</CardTitle>
          {BROWSER_MODE ? (
            <Badge variant="outline">browser preview</Badge>
          ) : available ? (
            <Badge variant="primary">v{check.data?.version} available</Badge>
          ) : (
            <Badge variant="success">up to date</Badge>
          )}
        </CardHeader>
        <CardContent className="flex flex-col gap-4">
          <div className="grid gap-4 sm:grid-cols-2">
            <Stat label="Installed version" value={`v${info.data?.version ?? "?"}`} />
            <Stat
              label="Latest on feed"
              value={
                BROWSER_MODE
                  ? "— (mock)"
                  : check.data
                    ? `v${check.data.version}`
                    : check.isLoading
                      ? "checking…"
                      : "not checked"
              }
            />
          </div>

          {check.data && !check.data.updateAvailable && !BROWSER_MODE ? (
            <p className="text-muted-foreground text-xs leading-relaxed">
              {check.data.notes
                ? `Feed said: ${check.data.notes}`
                : "You are running the newest published build."}
            </p>
          ) : null}

          {installer.state === "downloading" ? (
            <div className="flex flex-col gap-1.5">
              <div className="flex items-center justify-between text-[11px] tabular-nums">
                <span className="text-muted-foreground">Downloading update…</span>
                <span className="text-muted-foreground">
                  {total != null ? `${formatBytes(done)} / ${formatBytes(total)}` : formatBytes(done)}
                </span>
              </div>
              <Progress value={percent ?? undefined} className="h-1.5" />
            </div>
          ) : null}

          {installer.state === "installing" ? (
            <p className="text-muted-foreground animate-pulse text-xs">
              Applying the update — the launcher restarts when it is ready.
            </p>
          ) : null}

          <div className="flex flex-wrap items-center gap-2">
            {!BROWSER_MODE ? (
              <>
                <Button
                  size="sm"
                  variant="outline"
                  loading={check.isFetching || installer.check.isPending}
                  onClick={() => installer.check.mutate()}
                >
                  <RefreshCw className="size-4" /> Check for updates
                </Button>
                {available && installer.state !== "installing" ? (
                  installer.progress?.done ? (
                    <Button size="sm" loading={installer.install.isPending} onClick={() => installer.install.mutate()}>
                      <Zap className="size-4" /> Restart & install
                    </Button>
                  ) : (
                    <Button
                      size="sm"
                      loading={installer.download.isPending}
                      onClick={() => installer.download.mutate()}
                    >
                      <Zap className="size-4" /> Download v{check.data?.version}
                    </Button>
                  )
                ) : null}
              </>
            ) : null}
          </div>

          <p className="text-muted-foreground text-[11px] leading-relaxed">
            Updates are checked against a signed release feed; the signature is verified
            before anything is installed, so a tampered package can never load.
          </p>
        </CardContent>
      </Card>
    </>
  );
}

function DiagnosticsSection() {
  const info = useAppInfo();
  const nat = useNatProbe();
  const history = useSessionHistory();

  return (
    <>
      <Card>
        <CardHeader className="flex-row items-center justify-between">
          <CardTitle>About</CardTitle>
          <Badge variant="outline">{info.data?.version ?? "—"}</Badge>
        </CardHeader>
        <CardContent className="grid gap-4 sm:grid-cols-2">
          <Stat label="Tauri" value={info.data?.tauriVersion ?? "—"} />
          <Stat label="Rust toolchain" value={info.data?.rustVersion ?? "—"} />
          <Stat label="Platform" value={`${info.data?.os ?? "?"} · ${info.data?.arch ?? "?"}`} />
          <Stat label="Vault backend" value={info.data?.vaultBackend ?? "—"} />
          <Stat label="CurseForge" value={info.data?.curseforgeConfigured ? "configured" : "no key"} />
          <Stat label="Max server icon" value={formatBytes(info.data?.maxIconBytes ?? 0, 0)} />
        </CardContent>
      </Card>

      <Card>
        <CardHeader className="flex-row items-center justify-between">
          <CardTitle>Connectivity</CardTitle>
          <Button variant="outline" size="sm" onClick={() => void nat.refetch()} loading={nat.isFetching}>
            <RefreshCw className="size-4" /> Re-probe
          </Button>
        </CardHeader>
        <CardContent className="flex flex-col gap-3">
          <div className="flex flex-wrap items-center gap-2">
            <Badge variant={nat.data?.canHostDirect ? "success" : "warning"}>
              {nat.data?.behavior.replace(/_/g, " ") ?? "unknown"}
            </Badge>
            {nat.data?.publicAddress ? <Badge variant="outline">{nat.data.publicAddress}</Badge> : null}
          </div>
          <p className="text-muted-foreground text-xs leading-relaxed">
            {nat.data?.message ?? "Probing your network…"}
          </p>
          <Separator />
          <div className="grid gap-4 sm:grid-cols-3">
            <Stat label="Recent sessions" value={history.data?.joins.length ?? 0} />
            <Stat label="P2P tunnels" value={history.data?.p2p.length ?? 0} />
            <Stat
              label="Relayed"
              value={`${Math.round((history.data?.relayRatio ?? 0) * 100)}%`}
            />
          </div>
          {(history.data?.joins ?? []).slice(0, 5).map((entry, index) => (
            <div key={`${entry.serverName}-${index}`} className="flex items-center gap-3 text-xs">
              <Badge variant={entry.outcome === "success" ? "success" : "warning"}>{entry.outcome}</Badge>
              <span className="truncate">{entry.serverName}</span>
              <span className="text-muted-foreground ml-auto shrink-0">
                {formatRelative(entry.joinedAt)}
              </span>
            </div>
          ))}
        </CardContent>
      </Card>

      <Card>
        <CardHeader>
          <CardTitle className="flex items-center gap-2">
            <ScrollText className="size-4" /> Launcher log
          </CardTitle>
        </CardHeader>
        <CardContent>
          <LogTail />
        </CardContent>
      </Card>

      <Card>
        <CardHeader>
          <CardTitle className="flex items-center gap-2">
            <Users className="size-4" /> Open source
          </CardTitle>
        </CardHeader>
        <CardContent className="text-muted-foreground flex flex-col gap-2 text-xs leading-relaxed">
          <p>
            SXMLAUNCHER is MIT licensed. The Rust backend is in <code>src-tauri/src</code>,
            organized by domain: <code>auth</code>, <code>instances</code>, <code>mods</code>,{" "}
            <code>network</code> and <code>store</code>.
          </p>
          <p>
            Runtime sessions: {formatDuration(0)} — diagnostic counters only, nothing is uploaded
            unless anonymous diagnostics are enabled.
          </p>
        </CardContent>
      </Card>
    </>
  );
}

/** Last lines of the launcher log, refreshed on demand. */
function LogTail() {
  const [name, setName] = useState("launcher");
  const [lines, setLines] = useState<string[]>([]);
  const [loading, setLoading] = useState(false);

  const load = async () => {
    setLoading(true);
    try {
      setLines(await systemService.logTail(name, 120));
    } catch (error) {
      toast.error(error, "Could not read the log");
    } finally {
      setLoading(false);
    }
  };

  return (
    <div className="flex flex-col gap-3">
      <div className="flex items-center gap-2">
        <Input value={name} onChange={(event) => setName(event.target.value)} className="h-9 w-40 text-xs" />
        <Button variant="outline" size="sm" onClick={() => void load()} loading={loading}>
          <ScrollText className="size-4" /> Load tail
        </Button>
      </div>
      <pre className="max-h-64 overflow-auto rounded-lg border border-white/8 bg-black/40 p-3 text-[11px] leading-relaxed">
        {lines.length > 0 ? lines.join("\n") : "Press “Load tail” to read the last lines."}
      </pre>
    </div>
  );
}
