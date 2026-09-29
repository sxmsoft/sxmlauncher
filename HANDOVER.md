# SXMLAUNCHER — handover notes

Status snapshot for the incoming developer. Read together with
[README.md](README.md) (architecture, setup, CI) and [.env.example](.env.example).

## Where the code is

* `main` is the only branch. It holds the full Tauri 2 rewrite: the old PR
  stack (#2 → #10) plus the handover cleanup (#11). All of those PRs were closed
  as "consolidated into main" and their branches were deleted from GitHub.
* A few commits that were **not** in the stack are archived in the owner's
  offline backup bundle (ask the owner if you need them; they are not on GitHub):
  * P2P hardening, 5 commits (old PR #7, tip `cf89fca`): stricter share-code
    validation, Quick Play joins on modern clients, relay fallback when a
    strict-NAT direct tunnel stalls, Redis URL redaction.
  * sx.acc / UI fixes, 2 commits (old PR #9 tip `acb2188`, `fcaeb16`): OS trust
    store + system proxy for sx.acc refresh, SPA consent page, scrollable login
    dialog, app-owned context menu. These are partial attempts at issues 1–3 below.
  * Activity progress tweak, 1 commit (old PR #3 tip `4e5ecfb`).

## Known open issues

1. **Login dialog — create-account section is not scrollable.** On small
   windows the sx.acc registration form overflows the dialog and the submit
   button can't be reached. Fix in `src/components/account/login-dialog.tsx`
   (make the dialog body `overflow-y-auto` with a `max-h` tied to the viewport,
   or use `components/ui/scroll-area.tsx`, which is currently unused).
   An archived commit (`acb2188`, see above) has an attempt.
2. **Disable the Tauri WebView default right-click menu** and replace it with
   app-owned menus (instance card, library, text inputs keep copy/paste).
   Needs a global `contextmenu` handler in the frontend plus, on Windows,
   WebView2's `AreDefaultContextMenusEnabled = false`
   (`src-tauri/src/lib.rs`). Archived commits `acb2188` / `fcaeb16` have an attempt
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

* The working tree contains **no secrets** (gitleaks + trufflehog + manual
  review). All secrets are read from env/Settings.
* An Ely.by OAuth client secret that once sat in an obsolete pre-rewrite
  branch was scrubbed from history (`***REMOVED***`), and author e-mails were
  normalised to the GitHub noreply address. The owner must still **rotate** that
  secret at <https://account.ely.by/dev/applications> and ask GitHub Support to
  purge cached `refs/pull/*` views. The current code does not use a secret with
  the default public client `sxmlauncher3`.
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
