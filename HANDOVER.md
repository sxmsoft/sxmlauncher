# SXMLAUNCHER — handover notes

Status snapshot for the incoming developer. Read together with
[README.md](README.md) (architecture, setup, CI) and [.env.example](.env.example).

## Where the code is

* The PR branches form a **stack**; the latest complete code is
  `cursor/fix-modpack-launch-exit-8ca3` (PR #10). `handover/cleanup` is cut
  from it and only adds docs, config hygiene and log redaction.
* `main` still contains only the pre-rewrite reset commit. Nothing of the
  Tauri 2 rewrite is merged yet.
* Suggested merge order: #2 → #3 → #4 → #5 → #6 → (`cursor/misty-fixes-windows-build`)
  → #8 → #9 → #10 → handover PR, then rebase #7 and the two unmerged #9 commits
  (see below) on top.

## In-flight PRs / branches

| PR | Branch | Base | One-line summary |
|---|---|---|---|
| #2 | `cursor/sxmlauncher-rewrite-a8a2` | `main` | Tauri 2 + React launcher rewrite (vertical slice). |
| #3 | `cursor/nebula-vault-ui-3cab` | #2 | "Nebula Vault" UI. Tip commit 05a6886 (Activity progress only while work runs) was **not** carried into the stack. |
| #4 | `cursor/fix-oauth-providers-9aa8` | #3 | Fix Ely.by and Microsoft browser sign-in. |
| #5 | `cursor/misty-visual-redesign-f4d5` | #4 | Charcoal play board redesign. |
| #6 | `cursor/fix-production-bugs-630f` | #5 | Activity, wallpaper, Ely.by skin, Xbox login, modpack install fixes. |
| — | `cursor/misty-fixes-windows-build` | (#6) | No PR; Windows-build fixes that #7 and #8 are based on. |
| #7 | `cursor/p2p-host-join-1794` | misty-fixes | Harden P2P host/share code/join, Quick Play joins, relay fallback on strict NAT, Redis URL redaction. **5 commits not in the #10 stack** — needs a rebase. |
| #8 | `cursor/loader-icons-discord-presence-7b8b` | misty-fixes | New loader icons + Discord Rich Presence. |
| #9 | `cursor/sxacc-account-b685` | #8 | sx.acc login/registration + authlib launch. Its **last 2 commits (278b020, c81e0d9: sx.acc refresh/OS trust store, SPA consent page, scrollable login dialog, app-owned context menu) are not in #10** — unreviewed partial fixes for the open issues below. |
| #10 | `cursor/fix-modpack-launch-exit-8ca3` | #9 | Fix modpacks exiting with code 1 on legacy `${classpath}` placeholders; sx.acc v2 routes; SXMWARE credits. |
| #1 (closed) | `cursor/fix-critical-launcher-bugs-4a9e` | `main` | Pre-rewrite launcher fixes. Obsolete. **Contains a leaked secret in history — see "Security follow-ups".** Delete after the history scrub. |

## Known open issues

1. **Login dialog — create-account section is not scrollable.** On small
   windows the sx.acc registration form overflows the dialog and the submit
   button can't be reached. Fix in `src/components/account/login-dialog.tsx`
   (make the dialog body `overflow-y-auto` with a `max-h` tied to the viewport,
   or use `components/ui/scroll-area.tsx`, which is currently unused).
   Commit 278b020 on #9 has an attempt.
2. **Disable the Tauri WebView default right-click menu** and replace it with
   app-owned menus (instance card, library, text inputs keep copy/paste).
   Needs a global `contextmenu` handler in the frontend plus, on Windows,
   WebView2's `AreDefaultContextMenusEnabled = false`
   (`src-tauri/src/lib.rs`). Commits 278b020 / c81e0d9 on #9 have an attempt
   (`src/components/ui/app-context-menu.tsx`).
3. **Auth refresh hardening** (`src-tauri/src/auth/mod.rs`, `auth/sxacc.rs`, `auth/msa.rs`):
   * Don't force a refresh when the access token is still valid —
     `refresh_account` currently expires the stored token on purpose;
     launch/Play should only refresh when `is_expired(EXPIRY_SKEW_SECS)`.
   * Use short, explicit timeouts for refresh calls (the shared auth client
     uses 30 s; a hung refresh blocks Play). Surface a retryable error.
   * **Never clear tokens / mark signed-out on transport errors or timeouts**
     — only on a definitive `invalid_grant` / 401 from the provider
     (`AppError::Unauthorized`). Audit every provider's error mapping so a
     timeout can't become `Unauthorized`.
   * The concurrent-refresh waiter polls for up to 10 s; keep it bounded but
     return the stored (still valid) token when possible.
4. **Secrets entered in Settings are stored in plain text** (SQLite +
   `settings.json` in app data) and returned to the UI: CurseForge key,
   Ely.by client secret, host password, Redis URL credentials. Env overrides
   are merged in at load, so saving Settings can persist an env-provided value.
   Move these into `auth::vault` and expose only `has*` booleans.
5. **P2P signaling is unauthenticated.** `TODO(security)` in
   `src-tauri/src/state.rs` / `network/session.rs`: generate and persist an
   Ed25519 key pair and sign signaling envelopes / listings. The default
   directory is a *public* MQTT broker (`broker.emqx.io`), so anyone can read
   listings and inject signals.
6. **Updater / infrastructure URLs point at domains/repos to verify**:
   `tauri.conf.json` updater endpoints use `github.com/sxmlauncher/sxmlauncher`
   and `updates.sxmlauncher.dev`; the default relay is
   `wss://relay.sxmlauncher.dev`; `Cargo.toml` `repository` also points at
   `sxmlauncher/sxmlauncher`. The code lives at `sxmsoft/sxmlauncher`. Confirm
   who owns these before the first public release (the updater URL is
   compiled into the binary).
7. **Release workflow is gated** by typing `LINUX-QA-PASSED` and builds
   Windows only (`release.yml`). Decide whether to keep the gate and add Linux
   bundles to releases.

## Security follow-ups (owner action)

* The working tree of `handover/cleanup` contains **no secrets** (gitleaks +
  trufflehog + manual review). All secrets are read from env/Settings.
* History still contains an **Ely.by OAuth client secret** (branch
  `cursor/fix-critical-launcher-bugs-4a9e`, commit `e1f2ec0`,
  `src-tauri/src/config.rs`). Rotate it at
  <https://account.ely.by/dev/applications> and then scrub history (plan in
  the handover PR description). The current code does not use a secret with
  the default public client `sxmlauncher3`.
* Four early commits carry the owner's personal e-mail as author metadata;
  fix with a `.mailmap` + filter-repo during the same scrub if desired.
* GitHub Actions only use `secrets.TAURI_SIGNING_PRIVATE_KEY` and
  `secrets.TAURI_SIGNING_PRIVATE_KEY_PASSWORD`. Keep the private key in the
  gitignored `.keys/` folder and in the repository secrets only.

## Public-by-design values kept in code

| Value | Where | Why it is fine |
|---|---|---|
| Microsoft client id `00000000402b5328` | `config.rs`, `msa.rs`, `tauri.conf.json` deep link | Public OAuth client (no secret, PKCE / `ms-xal-` redirect); used by many launchers. Overridable via `SXML_MSA_CLIENT_ID`. |
| Ely.by client id `sxmlauncher3` | `config.rs`, `elyby.rs` | Public desktop application, no secret, no redirect. |
| sx.acc OAuth client id `sxmlauncher` + redirect `sxmlauncher://auth/callback` | `auth/sxacc.rs` | Public client identifier for a PKCE/deep-link flow; no secret. |
| Tauri updater `pubkey` | `tauri.conf.json` | Public half of the signing key; required by clients to verify updates. |
| STUN servers, `broker.emqx.io`, relay URL | `config.rs`, `network/mqtt.rs` | Public endpoints, no credentials. |
| Discord application id | not in code (env/Settings) | Would be public anyway (snowflake shown to every Discord client). |
