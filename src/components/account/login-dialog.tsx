import { useEffect, useState } from "react";

import { useQueryClient } from "@tanstack/react-query";
import { Check, ExternalLink, KeyRound, ShieldCheck, UserPlus } from "lucide-react";
import { useTranslation } from "react-i18next";

import { BrandMark } from "@/components/brand/logo";

import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card } from "@/components/ui/card";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Field, Input } from "@/components/ui/input";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import {
  useBeginLogin,
  useCompleteLogin,
  useElyByPasswordLogin,
  useLoginOffline,
  useSettings,
  useSxAccCapabilities,
  useSxAccPasswordLogin,
  useSxAccRegister,
} from "@/hooks/queries";
import { qk } from "@/lib/query-client";
import { openExternal } from "@/lib/window";
import { accountService } from "@/services";
import { useLoginStore } from "@/stores/login";
import { toast } from "@/stores/ui";
import type { AccountSummary, DeviceCodePrompt } from "@/types/account";

type Tab = "microsoft" | "elyby" | "sxacc" | "offline";

/**
 * Sign-in for Microsoft, Ely.by, sx.acc, and offline.
 *
 * Microsoft (the public Minecraft client) and Ely.by both open the system
 * browser on the provider's own login page — never an embedded webview, which
 * is what gets OAuth apps blocked. Microsoft returns through its `ms-xal`
 * callback. Ely.by's public desktop client has no redirect, so that button
 * opens the Ely.by device-code page and the backend polls until it completes.
 * A custom Ely.by web app (client secret set) still uses the loopback
 * redirect. Offline profiles need no network and get a deterministic
 * `OfflinePlayer:<name>` UUID.
 */
export function LoginDialog({
  open,
  onOpenChange,
  defaultTab = "microsoft",
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  defaultTab?: Tab;
}) {
  const [tab, setTab] = useState<Tab>(defaultTab);
  const [nickname, setNickname] = useState("");
  const [elyUsername, setElyUsername] = useState("");
  const [elyPassword, setElyPassword] = useState("");
  const [sxUsernameLogin, setSxUsernameLogin] = useState("");
  const [sxPassword, setSxPassword] = useState("");
  const [sxRegEmail, setSxRegEmail] = useState("");
  const [sxRegPassword, setSxRegPassword] = useState("");
  const [sxUsername, setSxUsername] = useState("");

  // Individual selectors: the actions are stable, so the cleanup effect below
  // runs only when the dialog actually opens or closes.
  const pending = useLoginStore((state) => state.pending);
  const error = useLoginStore((state) => state.error);
  const cancelLogin = useLoginStore((state) => state.cancel);
  const resetLogin = useLoginStore((state) => state.reset);

  const begin = useBeginLogin();
  const complete = useCompleteLogin();
  const offline = useLoginOffline();
  const elyby = useElyByPasswordLogin();
  const sxLogin = useSxAccPasswordLogin();
  const sxRegister = useSxAccRegister();
  const settings = useSettings();
  const sxBase = settings.data?.sxaccBaseUrl?.trim() ?? "";
  const sxCaps = useSxAccCapabilities(sxBase, open && tab === "sxacc" && sxBase.length > 0);

  // Closing must release the loopback listener of an unfinished attempt.
  useEffect(() => {
    if (open) resetLogin();
    else void cancelLogin();
  }, [open, cancelLogin, resetLogin]);

  const onSignedIn = () => onOpenChange(false);

  const startBrowserFlow = (provider: "microsoft" | "ely_by" | "sx_acc") => {
    begin.mutate(provider, {
      onSuccess: async (info) => {
        try {
          await openExternal(info.authorizeUrl);
        } catch (error) {
          await cancelLogin();
          const message = error instanceof Error ? error.message : String(error);
          useLoginStore.getState().setError(message);
          toast.error(error, "Could not open the sign-in page");
          return;
        }
        complete.mutate(info.loginId, { onSuccess: onSignedIn });
      },
    });
  };

  const { t } = useTranslation();
  const waiting = pending != null;

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="flex h-[min(88vh,860px)] w-[min(96vw,920px)] max-w-[920px] flex-col overflow-hidden p-0">
        <div className="grid h-full min-h-0 grid-cols-1 md:grid-cols-[220px_minmax(0,1fr)]">
          <aside className="relative hidden flex-col justify-between overflow-hidden border-r border-white/8 bg-[#121214] p-6 md:flex">
            <div
              aria-hidden
              className="pointer-events-none absolute inset-0"
              style={{
                background:
                  "radial-gradient(ellipse at 30% 20%, var(--accent-glow), transparent 60%), linear-gradient(180deg, #1a1a1e, #101012)",
              }}
            />
            <div className="relative">
              <BrandMark slot={64} className="size-10" />
            </div>
            <div className="relative">
              <p className="text-lg font-semibold tracking-[0.12em]">SXMLAUNCHER</p>
              <p className="mt-2 text-sm leading-relaxed text-white/70">{t("login.aside")}</p>
            </div>
          </aside>

          <div className="min-h-0 overflow-y-auto overscroll-contain p-6">
        <DialogHeader>
          <DialogTitle>{t("login.title")}</DialogTitle>
          <DialogDescription>{t("login.body")}</DialogDescription>
        </DialogHeader>

        <Tabs value={tab} onValueChange={(value) => setTab(value as Tab)}>
          <TabsList className="grid w-full grid-cols-2 md:grid-cols-4">
            <TabsTrigger value="microsoft" className="justify-center">
              <ShieldCheck /> {t("login.microsoft")}
            </TabsTrigger>
            <TabsTrigger value="elyby" className="justify-center">
              <KeyRound /> {t("login.elyby")}
            </TabsTrigger>
            <TabsTrigger value="sxacc" className="justify-center">
              <KeyRound /> {t("login.sxacc")}
            </TabsTrigger>
            <TabsTrigger value="offline" className="justify-center">
              <UserPlus /> {t("login.offline")}
            </TabsTrigger>
          </TabsList>

          <TabsContent value="microsoft" className="flex flex-col gap-3">
            <p className="text-muted-foreground text-xs leading-relaxed">{t("login.microsoftBody")}</p>
            <Button
              className="rounded-full"
              onClick={() => startBrowserFlow("microsoft")}
              loading={complete.isPending && tab === "microsoft"}
            >
              <ExternalLink /> {t("login.continue")}
            </Button>
            {waiting && tab === "microsoft" ? (
              <p className="text-muted-foreground text-xs">{t("login.waiting")}</p>
            ) : null}
            <DeviceCodeSection onSuccess={onSignedIn} />
          </TabsContent>

          <TabsContent value="sxacc" className="flex flex-col gap-4">
            <p className="text-muted-foreground text-xs leading-relaxed">{t("login.sxaccBody")}</p>
            {!sxBase ? (
              <p className="text-xs leading-relaxed text-[var(--destructive)]">{t("login.sxaccMissing")}</p>
            ) : null}
            {sxCaps.data?.message ? (
              <p className="text-muted-foreground text-xs leading-relaxed">{sxCaps.data.message}</p>
            ) : null}
            <Button
              variant="outline"
              className="rounded-full"
              disabled={!sxBase || sxCaps.data?.oauth === false}
              onClick={() => startBrowserFlow("sx_acc")}
              loading={complete.isPending && tab === "sxacc"}
            >
              <ExternalLink /> {t("login.sxaccBrowser")}
            </Button>
            {waiting && tab === "sxacc" ? (
              <p className="text-muted-foreground text-xs">{t("login.waiting")}</p>
            ) : null}
            <Card className="flex flex-col gap-3 rounded-2xl p-4">
              <span className="text-xs font-medium">{t("login.signIn")}</span>
              <Field label={t("login.username")} htmlFor="sx-user" hint={t("login.nicknameHint")}>
                <Input
                  id="sx-user"
                  value={sxUsernameLogin}
                  onChange={(event) => setSxUsernameLogin(event.target.value)}
                  placeholder="Ada"
                  maxLength={16}
                  autoComplete="username"
                />
              </Field>
              <Field label={t("login.password")} htmlFor="sx-pass">
                <Input
                  id="sx-pass"
                  type="password"
                  value={sxPassword}
                  onChange={(event) => setSxPassword(event.target.value)}
                  autoComplete="current-password"
                />
              </Field>
              <Button
                size="sm"
                disabled={
                  !sxBase ||
                  sxUsernameLogin.trim().length < 3 ||
                  !sxPassword ||
                  sxCaps.data?.password === false
                }
                loading={sxLogin.isPending}
                onClick={() =>
                  sxLogin.mutate(
                    { username: sxUsernameLogin.trim(), password: sxPassword },
                    { onSuccess: onSignedIn },
                  )
                }
              >
                {t("login.signIn")}
              </Button>
            </Card>
            <Card className="flex flex-col gap-3 rounded-2xl p-4">
              <span className="text-xs font-medium">{t("login.sxaccRegister")}</span>
              <p className="text-muted-foreground text-xs leading-relaxed">{t("login.sxaccRegisterHint")}</p>
              <Field label={t("login.sxaccEmail")} htmlFor="sx-reg-email">
                <Input
                  id="sx-reg-email"
                  value={sxRegEmail}
                  onChange={(event) => setSxRegEmail(event.target.value)}
                  placeholder="ada@example.com"
                  autoComplete="email"
                />
              </Field>
              <Field label={t("login.sxaccUsername")} htmlFor="sx-reg-user" hint={t("login.nicknameHint")}>
                <Input
                  id="sx-reg-user"
                  value={sxUsername}
                  onChange={(event) => setSxUsername(event.target.value)}
                  placeholder="Ada"
                  maxLength={16}
                  autoComplete="off"
                />
              </Field>
              <Field label={t("login.password")} htmlFor="sx-reg-pass">
                <Input
                  id="sx-reg-pass"
                  type="password"
                  value={sxRegPassword}
                  onChange={(event) => setSxRegPassword(event.target.value)}
                  autoComplete="new-password"
                />
              </Field>
              <Button
                size="sm"
                disabled={
                  !sxBase ||
                  !sxRegEmail.includes("@") ||
                  sxRegPassword.length < 8 ||
                  sxUsername.trim().length < 3 ||
                  sxCaps.data?.register === false
                }
                loading={sxRegister.isPending}
                onClick={() =>
                  sxRegister.mutate(
                    {
                      email: sxRegEmail.trim(),
                      password: sxRegPassword,
                      username: sxUsername.trim(),
                    },
                    { onSuccess: onSignedIn },
                  )
                }
              >
                {t("login.sxaccRegister")}
              </Button>
            </Card>
            {sxCaps.data?.device === false ? null : (
              <DeviceCodeSection
                onSuccess={onSignedIn}
                begin={() => accountService.beginSxAccDevice()}
                complete={(prompt) =>
                  accountService.completeSxAccDevice({
                    ...prompt,
                    tokenUrl: "tokenUrl" in prompt ? String(prompt.tokenUrl) : "",
                  })
                }
              />
            )}
          </TabsContent>

          <TabsContent value="elyby" className="flex flex-col gap-4">
            <p className="text-muted-foreground text-xs leading-relaxed">{t("login.elyBody")}</p>
            <Button
              variant="outline"
              className="rounded-full"
              onClick={() => startBrowserFlow("ely_by")}
              loading={complete.isPending && tab === "elyby"}
            >
              <ExternalLink /> {t("login.elyBrowser")}
            </Button>
            <Card className="flex flex-col gap-3 rounded-2xl p-4">
              <span className="text-xs font-medium">{t("login.elyPassword")}</span>
              <Field label={t("login.username")} htmlFor="ely-user">
                <Input
                  id="ely-user"
                  value={elyUsername}
                  onChange={(event) => setElyUsername(event.target.value)}
                  placeholder="PlayerName"
                  autoComplete="username"
                />
              </Field>
              <Field label={t("login.password")} htmlFor="ely-pass">
                <Input
                  id="ely-pass"
                  type="password"
                  value={elyPassword}
                  onChange={(event) => setElyPassword(event.target.value)}
                  autoComplete="current-password"
                />
              </Field>
              <Button
                size="sm"
                disabled={!elyUsername || !elyPassword}
                loading={elyby.isPending}
                onClick={() =>
                  elyby.mutate(
                    { username: elyUsername, password: elyPassword },
                    { onSuccess: onSignedIn },
                  )
                }
              >
                {t("login.signIn")}
              </Button>
            </Card>
          </TabsContent>

          <TabsContent value="offline" className="flex flex-col gap-3">
            <p className="text-muted-foreground text-xs leading-relaxed">{t("login.offlineBody")}</p>
            <Field
              label={t("login.nickname")}
              htmlFor="offline-name"
              hint={t("login.nicknameHint")}
            >
              <Input
                id="offline-name"
                value={nickname}
                onChange={(event) => setNickname(event.target.value)}
                placeholder="Steve"
                maxLength={16}
                autoComplete="off"
              />
            </Field>
            <Button
              disabled={nickname.trim().length < 3}
              loading={offline.isPending}
              onClick={() => offline.mutate(nickname.trim(), { onSuccess: onSignedIn })}
            >
              <Check /> {t("login.createOffline")}
            </Button>
          </TabsContent>
        </Tabs>

        {error ? (
          <p className="text-[var(--destructive)] mt-4 text-xs leading-relaxed">{error}</p>
        ) : null}
          </div>
        </div>
      </DialogContent>
    </Dialog>
  );
}

/** Device-code grant, for machines where opening a browser is not possible. */
function DeviceCodeSection({
  onSuccess,
  begin = () => accountService.beginDeviceCode(),
  complete = (prompt) => accountService.completeDeviceCode(prompt),
}: {
  onSuccess?: () => void;
  begin?: () => Promise<DeviceCodePrompt>;
  complete?: (prompt: DeviceCodePrompt) => Promise<AccountSummary>;
}) {
  const deviceCode = useLoginStore((state) => state.deviceCode);
  const busy = useLoginStore((state) => state.busy);
  const setDeviceCode = useLoginStore((state) => state.setDeviceCode);
  const setError = useLoginStore((state) => state.setError);
  const setBusy = useLoginStore((state) => state.setBusy);
  const [starting, setStarting] = useState(false);
  const client = useQueryClient();
  const { t } = useTranslation();

  const start = async () => {
    setStarting(true);
    setBusy(true);
    try {
      const prompt = await begin();
      setDeviceCode(prompt);
      await openExternal(prompt.verificationUri);
    } catch (error) {
      setError(error instanceof Error ? error.message : String(error));
      toast.error(error, "Could not start the device-code sign-in");
    } finally {
      setBusy(false);
      setStarting(false);
    }
  };

  const finish = async () => {
    if (!deviceCode) return;
    setBusy(true);
    try {
      const account = await complete(deviceCode);
      client.setQueryData(qk.activeAccount, account);
      void client.invalidateQueries({ queryKey: qk.accounts });
      toast.success(`Signed in as ${account.username}`);
      onSuccess?.();
    } catch (error) {
      setError(error instanceof Error ? error.message : String(error));
    } finally {
      setBusy(false);
    }
  };

  if (!deviceCode) {
    return (
      <Button variant="ghost" size="sm" onClick={() => void start()} loading={starting}>
        {t("login.device")}
      </Button>
    );
  }

  return (
    <Card className="flex flex-col gap-2 rounded-2xl p-4">
      <span className="text-xs font-medium">
        {t("login.deviceHint", { url: deviceCode.verificationUri })}
      </span>
      <Badge variant="primary" className="w-fit px-3 py-1 text-sm tracking-[0.2em]">
        {deviceCode.userCode}
      </Badge>
      <Button size="sm" onClick={() => void finish()} loading={busy}>
        {t("login.deviceDone")}
      </Button>
    </Card>
  );
}
