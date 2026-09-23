import { useEffect, useState } from "react";

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
} from "@/hooks/queries";
import { openExternal } from "@/lib/window";
import { accountService } from "@/services";
import { useLoginStore } from "@/stores/login";
import { toast } from "@/stores/ui";

type Tab = "microsoft" | "elyby" | "offline";

/**
 * Sign-in for all three providers.
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

  // Closing must release the loopback listener of an unfinished attempt.
  useEffect(() => {
    if (open) resetLogin();
    else void cancelLogin();
  }, [open, cancelLogin, resetLogin]);

  const onSignedIn = () => onOpenChange(false);

  const startBrowserFlow = (provider: "microsoft" | "ely_by") => {
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
      <DialogContent className="w-[min(96vw,920px)] max-w-[920px] overflow-hidden p-0">
        <div className="grid min-h-[520px] grid-cols-[220px_minmax(0,1fr)]">
          <aside className="relative flex flex-col justify-between overflow-hidden border-r border-white/8 bg-[#121214] p-6">
            <div
              aria-hidden
              className="pointer-events-none absolute inset-0"
              style={{
                background:
                  "radial-gradient(ellipse at 30% 20%, var(--accent-glow), transparent 60%), linear-gradient(180deg, #1a1a1e, #101012)",
              }}
            />
            <div className="relative">
              <BrandMark className="size-10" />
            </div>
            <div className="relative">
              <p className="text-lg font-semibold tracking-[0.12em]">SXMLAUNCHER</p>
              <p className="mt-2 text-sm leading-relaxed text-white/70">{t("login.aside")}</p>
            </div>
          </aside>

          <div className="p-6">
        <DialogHeader>
          <DialogTitle>{t("login.title")}</DialogTitle>
          <DialogDescription>{t("login.body")}</DialogDescription>
        </DialogHeader>

        <Tabs value={tab} onValueChange={(value) => setTab(value as Tab)}>
          <TabsList className="grid w-full grid-cols-3">
            <TabsTrigger value="microsoft" className="justify-center">
              <ShieldCheck /> {t("login.microsoft")}
            </TabsTrigger>
            <TabsTrigger value="elyby" className="justify-center">
              <KeyRound /> {t("login.elyby")}
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
            <DeviceCodeSection />
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
function DeviceCodeSection() {
  const deviceCode = useLoginStore((state) => state.deviceCode);
  const busy = useLoginStore((state) => state.busy);
  const setDeviceCode = useLoginStore((state) => state.setDeviceCode);
  const setError = useLoginStore((state) => state.setError);
  const setBusy = useLoginStore((state) => state.setBusy);
  const [starting, setStarting] = useState(false);
  const { t } = useTranslation();

  const begin = async () => {
    setStarting(true);
    setBusy(true);
    try {
      const prompt = await accountService.beginDeviceCode();
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
      const account = await accountService.completeDeviceCode(deviceCode);
      toast.success(`Signed in as ${account.username}`);
    } catch (error) {
      setError(error instanceof Error ? error.message : String(error));
    } finally {
      setBusy(false);
    }
  };

  if (!deviceCode) {
    return (
      <Button variant="ghost" size="sm" onClick={() => void begin()} loading={starting}>
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
