import { useEffect, useState } from "react";

import { Check, ExternalLink, KeyRound, ShieldCheck, UserPlus } from "lucide-react";

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
 * Microsoft uses OAuth2 + PKCE against a loopback listener (or device code).
 * Ely.by prefers username/password (Authlib); browser OAuth needs a registered
 * application on account.ely.by whose client id / secret / redirect URI match
 * Settings → Accounts exactly.
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
        await openExternal(info.authorizeUrl);
        complete.mutate(info.loginId, { onSuccess: onSignedIn });
      },
    });
  };

  const waiting = pending != null;

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="max-w-xl">
        <DialogHeader>
          <DialogTitle>Sign in</DialogTitle>
          <DialogDescription>
            Tokens are stored in your operating system's credential vault. SXMLauncher
            never sees them again after sign-in.
          </DialogDescription>
        </DialogHeader>

        <Tabs value={tab} onValueChange={(value) => setTab(value as Tab)}>
          <TabsList>
            <TabsTrigger value="microsoft">
              <ShieldCheck /> Microsoft
            </TabsTrigger>
            <TabsTrigger value="elyby">
              <KeyRound /> Ely.by
            </TabsTrigger>
            <TabsTrigger value="offline">
              <UserPlus /> Offline
            </TabsTrigger>
          </TabsList>

          <TabsContent value="microsoft" className="flex flex-col gap-3">
            <p className="text-muted-foreground text-xs leading-relaxed">
              Opens your browser for the official Microsoft sign-in, then returns here.
              Required for online multiplayer and Realms.
            </p>
            <Button
              onClick={() => startBrowserFlow("microsoft")}
              loading={complete.isPending && tab === "microsoft"}
            >
              <ExternalLink /> Continue in browser
            </Button>
            {waiting && tab === "microsoft" ? (
              <p className="text-muted-foreground text-xs">
                Waiting for the browser to finish… you can close that tab once it says the
                sign-in is complete.
              </p>
            ) : null}
            <DeviceCodeSection onSignedIn={onSignedIn} />
          </TabsContent>

          <TabsContent value="elyby" className="flex flex-col gap-4">
            <p className="text-muted-foreground text-xs leading-relaxed">
              Ely.by accounts carry custom skins and capes. The launcher attaches the
              Authlib endpoint to the JVM automatically.
            </p>
            <Card className="flex flex-col gap-3 p-4">
              <span className="text-xs font-medium">Sign in with username and password</span>
              <Field label="Username" htmlFor="ely-user">
                <Input
                  id="ely-user"
                  value={elyUsername}
                  onChange={(event) => setElyUsername(event.target.value)}
                  placeholder="PlayerName"
                  autoComplete="username"
                />
              </Field>
              <Field label="Password" htmlFor="ely-pass">
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
                Sign in
              </Button>
            </Card>
            <div className="flex flex-col gap-2">
              <Button
                variant="outline"
                onClick={() => startBrowserFlow("ely_by")}
                loading={complete.isPending && tab === "elyby"}
              >
                <ExternalLink /> Sign in with Ely.by in the browser
              </Button>
              <p className="text-muted-foreground text-[11px] leading-relaxed">
                Browser sign-in needs an OAuth application registered at{" "}
                <button
                  type="button"
                  className="underline underline-offset-2"
                  onClick={() => void openExternal("https://account.ely.by/dev/applications")}
                >
                  account.ely.by/dev/applications
                </button>{" "}
                whose client id, secret, and redirect URI match Settings → Accounts. A
                mismatch shows “could not find application” / “uygulama bulunamadı” in the
                browser — use the password form above, or paste working credentials in
                Settings.
              </p>
            </div>
          </TabsContent>

          <TabsContent value="offline" className="flex flex-col gap-3">
            <p className="text-muted-foreground text-xs leading-relaxed">
              A local profile for singleplayer and LAN. The UUID is derived from the
              nickname, so the same name always maps to the same player.
            </p>
            <Field
              label="Nickname"
              htmlFor="offline-name"
              hint="3–16 characters: letters, numbers and underscores."
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
              <Check /> Create offline profile
            </Button>
          </TabsContent>
        </Tabs>

        {error ? (
          <p className="text-[var(--destructive)] mt-4 text-xs leading-relaxed">{error}</p>
        ) : null}
      </DialogContent>
    </Dialog>
  );
}

/** Device-code grant — opens the verification page and polls until complete. */
function DeviceCodeSection({ onSignedIn }: { onSignedIn: () => void }) {
  const deviceCode = useLoginStore((state) => state.deviceCode);
  const busy = useLoginStore((state) => state.busy);
  const setDeviceCode = useLoginStore((state) => state.setDeviceCode);
  const setError = useLoginStore((state) => state.setError);
  const setBusy = useLoginStore((state) => state.setBusy);
  const [starting, setStarting] = useState(false);

  const begin = async () => {
    setStarting(true);
    setBusy(true);
    setError(null);
    try {
      const prompt = await accountService.beginDeviceCode();
      setDeviceCode(prompt);
      await openExternal(prompt.verificationUri);
      // Poll automatically — no second click required.
      const account = await accountService.completeDeviceCode(prompt);
      toast.success(`Signed in as ${account.username}`);
      onSignedIn();
    } catch (error) {
      setError(error instanceof Error ? error.message : String(error));
      toast.error(error, "Could not complete the device-code sign-in");
    } finally {
      setBusy(false);
      setStarting(false);
    }
  };

  if (!deviceCode) {
    return (
      <Button variant="ghost" size="sm" onClick={() => void begin()} loading={starting || busy}>
        Can't open a browser? Use a device code
      </Button>
    );
  }

  return (
    <Card className="flex flex-col gap-2 p-4">
      <span className="text-xs font-medium">
        Enter this code at {deviceCode.verificationUri}
      </span>
      <Badge variant="primary" className="w-fit px-3 py-1 text-sm tracking-[0.2em]">
        {deviceCode.userCode}
      </Badge>
      <p className="text-muted-foreground text-[11px]">
        {busy ? "Waiting for Microsoft…" : "Waiting for you to enter the code in the browser…"}
      </p>
    </Card>
  );
}
