/** Account queries and the sign-in mutations for every provider. */

import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";

import { qk } from "@/lib/query-client";
import { accountService } from "@/services";
import { useLoginStore } from "@/stores/login";
import { toast } from "@/stores/ui";
import type { AccountProvider, AccountSummary, SkinModel } from "@/types/account";

export function useAccounts() {
  return useQuery({ queryKey: qk.accounts, queryFn: accountService.list });
}

export function useActiveAccount() {
  return useQuery({ queryKey: qk.activeAccount, queryFn: accountService.active });
}

export function useVaultBackend() {
  return useQuery({
    queryKey: qk.vaultBackend,
    queryFn: accountService.vaultBackend,
    staleTime: Infinity,
  });
}

function useAccountCacheWriter() {
  const client = useQueryClient();
  return (account: AccountSummary) => {
    client.setQueryData(qk.activeAccount, account);
    void client.invalidateQueries({ queryKey: qk.accounts });
  };
}

export function useSetActiveAccount() {
  const write = useAccountCacheWriter();
  return useMutation({
    mutationFn: (id: string) => accountService.setActive(id),
    onSuccess: write,
    onError: (error) => toast.error(error, "Could not switch account"),
  });
}

/** Guest mode: no network, deterministic offline UUID. */
export function useLoginOffline() {
  const write = useAccountCacheWriter();
  return useMutation({
    mutationFn: (username: string) => accountService.loginOffline(username),
    onSuccess: (account) => {
      write(account);
      toast.success(`Signed in as ${account.username}`, "Offline profile");
    },
    onError: (error) => toast.error(error, "Could not create the offline profile"),
  });
}

export function useElyByPasswordLogin() {
  const write = useAccountCacheWriter();
  return useMutation({
    mutationFn: ({ username, password }: { username: string; password: string }) =>
      accountService.loginElybyPassword(username, password),
    onSuccess: write,
    onError: (error) => toast.error(error, "Ely.by sign-in failed"),
  });
}

export function useSxAccPasswordLogin() {
  const write = useAccountCacheWriter();
  return useMutation({
    mutationFn: ({ email, password }: { email: string; password: string }) =>
      accountService.loginSxAccPassword(email, password),
    onSuccess: write,
    onError: (error) => toast.error(error, "sx.acc sign-in failed"),
  });
}

export function useSxAccRegister() {
  const write = useAccountCacheWriter();
  return useMutation({
    mutationFn: ({
      email,
      password,
      username,
    }: {
      email: string;
      password: string;
      username: string;
    }) => accountService.registerSxAcc(email, password, username),
    onSuccess: write,
    onError: (error) => toast.error(error, "Could not create the sx.acc account"),
  });
}

export function useSxAccCapabilities(baseUrl: string, enabled: boolean) {
  return useQuery({
    queryKey: [...qk.accounts, "sxacc", baseUrl],
    queryFn: accountService.sxaccCapabilities,
    enabled,
    staleTime: 15_000,
  });
}

/**
 * Kick off a browser sign-in and park it in the login store. The dialog then
 * opens `authorizeUrl` and calls {@link useCompleteLogin} to wait for the
 * loopback redirect.
 */
export function useBeginLogin() {
  return useMutation({
    mutationFn: (provider: AccountProvider) => accountService.beginLogin(provider),
    onMutate: () => {
      // `getState()` rather than a subscription: these are fire-and-forget
      // writes to a store this hook does not render from.
      useLoginStore.getState().setBusy(true);
      useLoginStore.getState().setError(null);
    },
    onSuccess: (pending) => useLoginStore.getState().setPending(pending),
    onError: (error) => {
      useLoginStore
        .getState()
        .setError(error instanceof Error ? error.message : String(error));
      toast.error(error, "Could not start the sign-in");
    },
    onSettled: () => useLoginStore.getState().setBusy(false),
  });
}

export function useCompleteLogin() {
  const client = useQueryClient();
  return useMutation({
    mutationFn: (loginId: string) => accountService.completeLogin(loginId),
    onMutate: () => useLoginStore.getState().setBusy(true),
    onSuccess: (account) => {
      client.setQueryData(qk.activeAccount, account);
      void client.invalidateQueries({ queryKey: qk.accounts });
      useLoginStore.getState().reset();
      toast.success(`Signed in as ${account.username}`);
      // Pull the current skin/cape straight away, so the account never shows a
      // stale default texture after a fresh sign-in.
      if (account.provider !== "offline") {
        accountService
          .refreshSkin(account.id)
          .then(() => {
            void client.invalidateQueries({ queryKey: qk.accounts });
            void client.invalidateQueries({ queryKey: qk.activeAccount });
          })
          .catch(() => {
            // Best effort: the account row already carries the login-time skin.
          });
      }
    },
    onError: (error) => {
      useLoginStore
        .getState()
        .setError(error instanceof Error ? error.message : String(error));
      toast.error(error, "Sign-in failed");
    },
    onSettled: () => useLoginStore.getState().setBusy(false),
  });
}

export function useSignOut() {
  const client = useQueryClient();
  return useMutation({
    mutationFn: (id: string) => accountService.signOut(id),
    onSuccess: () => {
      void client.invalidateQueries({ queryKey: qk.accounts });
      void client.invalidateQueries({ queryKey: qk.activeAccount });
      toast.info("Signed out", "The stored token was removed from the vault");
    },
    onError: (error) => toast.error(error, "Could not sign out"),
  });
}

/** Force a token refresh; useful when a session is near expiry. */
export function useRefreshAccount() {
  const client = useQueryClient();
  return useMutation({
    mutationFn: (id: string) => accountService.refresh(id),
    onSuccess: (account) => {
      client.setQueryData(qk.activeAccount, account);
      void client.invalidateQueries({ queryKey: qk.accounts });
      toast.success("Session refreshed");
    },
    onError: (error) => toast.error(error, "Could not refresh the session"),
  });
}

/** Re-read an account's skin/cape from its provider and persist it. */
export function useRefreshSkin() {
  const client = useQueryClient();
  return useMutation({
    mutationFn: (id: string) => accountService.refreshSkin(id),
    onSuccess: () => {
      void client.invalidateQueries({ queryKey: qk.accounts });
      void client.invalidateQueries({ queryKey: qk.activeAccount });
      toast.success("Skin updated", "Fetched the current texture from the provider");
    },
    onError: (error) => toast.error(error, "Could not reload the skin"),
  });
}

/**
 * Apply a user-picked PNG to an account.
 *
 * Microsoft accounts get a real upload through the official Mojang endpoint;
 * Ely.by accounts get local validation plus a deep link to the website's skin
 * page (its public API has no upload endpoint); offline profiles are rejected.
 * The outcome is written straight into the account caches so avatars update
 * without a refetch.
 */
export function useUploadSkin() {
  const client = useQueryClient();
  return useMutation({
    mutationFn: ({ id, model, png }: { id: string; model: SkinModel; png: Uint8Array }) =>
      accountService.uploadSkin(id, model, png),
    onSuccess: (outcome, variables) => {
      const existing = client.getQueryData<AccountSummary>(qk.activeAccount);
      if (existing?.id === variables.id) {
        client.setQueryData<AccountSummary>(qk.activeAccount, {
          ...existing,
          skin: outcome.skin,
        });
      }
      void client.invalidateQueries({ queryKey: qk.accounts });
      if (outcome.uploaded) {
        toast.success("Skin applied", "Uploaded to Mojang — the game picks it up next launch");
      } else if (outcome.message) {
        toast.info("Upload happens on the website", outcome.message);
      }
    },
    onError: (error) => toast.error(error, "Could not apply the skin"),
  });
}
