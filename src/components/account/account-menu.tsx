import { useState, type ReactNode } from "react";

import { Check, LogOut, Plus, RefreshCw } from "lucide-react";

import { AccountAvatar } from "@/components/account/account-avatar";
import { LoginDialog } from "@/components/account/login-dialog";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
  DialogTrigger,
} from "@/components/ui/dialog";
import { Separator } from "@/components/ui/feedback";
import { useAccounts, useActiveAccount, useRefreshAccount, useSetActiveAccount, useSignOut } from "@/hooks/queries";
import { cn, formatRelative } from "@/lib/utils";
import { sessionExpired, PROVIDER_LABEL } from "@/types/account";

/**
 * Account switcher.
 *
 * The trigger is whatever the caller passes (the sidebar chip, an "Add account"
 * button, …); the dialog lists every profile with its provider badge and lets
 * the user switch, refresh a session that is about to expire, or sign out.
 */
export function AccountMenu({ children }: { children?: ReactNode }) {
  const [open, setOpen] = useState(false);
  const [loginOpen, setLoginOpen] = useState(false);

  const accounts = useAccounts();
  const active = useActiveAccount();
  const setActive = useSetActiveAccount();
  const refresh = useRefreshAccount();
  const signOut = useSignOut();

  const activeId = active.data?.id ?? null;

  return (
    <>
      <Dialog open={open} onOpenChange={setOpen}>
        <DialogTrigger asChild>
          {children ?? (
            <button
              type="button"
              className="flex w-full items-center gap-2 rounded-lg border border-white/8 bg-black/20 p-2 text-left transition-colors hover:bg-white/6"
            >
              {active.data ? (
                <>
                  <AccountAvatar account={active.data} size={28} />
                  <span className="flex min-w-0 flex-1 flex-col">
                    <span className="truncate text-xs font-medium">{active.data.username}</span>
                    <span className="text-muted-foreground truncate text-[10px]">
                      {PROVIDER_LABEL[active.data.provider]}
                    </span>
                  </span>
                </>
              ) : (
                <>
                  <Plus className="size-4" />
                  <span className="text-xs font-medium">Add an account</span>
                </>
              )}
            </button>
          )}
        </DialogTrigger>

        <DialogContent>
          <DialogHeader>
            <DialogTitle>Accounts</DialogTitle>
            <DialogDescription>
              The active account supplies your username, UUID and skin at launch.
            </DialogDescription>
          </DialogHeader>

          <div className="flex flex-col gap-2">
            {(accounts.data ?? []).map((account) => {
              const isActive = account.id === activeId;
              const expiring = sessionExpired(account);
              return (
                <div
                  key={account.id}
                  className={cn(
                    "flex items-center gap-3 rounded-xl border p-3 transition-colors",
                    isActive ? "border-[color-mix(in_oklab,var(--primary)_45%,transparent)] bg-[color-mix(in_oklab,var(--primary)_10%,transparent)]" : "border-white/8 bg-black/20",
                  )}
                >
                  <AccountAvatar account={account} size={36} />
                  <div className="flex min-w-0 flex-1 flex-col gap-0.5">
                    <span className="flex items-center gap-2 text-sm font-medium">
                      {account.username}
                      <Badge variant={account.provider === "offline" ? "outline" : "primary"}>
                        {PROVIDER_LABEL[account.provider]}
                      </Badge>
                      {expiring ? <Badge variant="warning">refresh needed</Badge> : null}
                    </span>
                    <span className="text-muted-foreground text-[11px]">
                      used {formatRelative(account.lastUsedAt)}
                      {account.hasStoredCredentials ? " · token in vault" : ""}
                    </span>
                  </div>

                  <div className="flex items-center gap-1">
                    {account.provider !== "offline" ? (
                      <Button
                        variant="ghost"
                        size="icon-sm"
                        onClick={() => refresh.mutate(account.id)}
                        loading={refresh.isPending && refresh.variables === account.id}
                      >
                        <RefreshCw className="size-3.5" />
                        <span className="sr-only">Refresh session</span>
                      </Button>
                    ) : null}
                    {isActive ? (
                      <Badge variant="success">
                        <Check className="size-3" /> active
                      </Badge>
                    ) : (
                      <Button
                        size="sm"
                        variant="secondary"
                        onClick={() => setActive.mutate(account.id, { onSuccess: () => setOpen(false) })}
                        loading={setActive.isPending}
                      >
                        Use
                      </Button>
                    )}
                    <Button
                      variant="ghost"
                      size="icon-sm"
                      className="hover:text-[var(--destructive)]"
                      onClick={() => signOut.mutate(account.id)}
                    >
                      <LogOut className="size-3.5" />
                      <span className="sr-only">Sign out</span>
                    </Button>
                  </div>
                </div>
              );
            })}

            {accounts.data?.length === 0 ? (
              <p className="text-muted-foreground text-sm">
                No accounts yet — add one to play online, or an offline profile for
                singleplayer.
              </p>
            ) : null}
          </div>

          <Separator className="my-4" />

          <Button
            variant="outline"
            className="w-full"
            onClick={() => {
              setOpen(false);
              setLoginOpen(true);
            }}
          >
            <Plus /> Add an account
          </Button>
        </DialogContent>
      </Dialog>

      <LoginDialog open={loginOpen} onOpenChange={setLoginOpen} />
    </>
  );
}
