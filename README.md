# SXMLAUNCHER

**SXMLAUNCHER** (by **SXMWARE**) is a desktop Minecraft launcher built with
**Tauri 2** (Rust backend) and **React 19** (TypeScript frontend). It manages
isolated game instances, installs Modrinth / CurseForge modpacks, signs players
in with Microsoft, Ely.by, sx.acc or offline accounts, and hosts / joins
worlds peer-to-peer with a short share code.

> Handing the project over? Read [HANDOVER.md](HANDOVER.md) for open issues
> and the list of in-flight branches.

---

## Features at a glance

| Area | What it does |
|---|---|
| Instances | Isolated game dirs (vanilla, Fabric, Quilt, Forge, NeoForge), per-instance memory / Java, shared asset + library store, managed Temurin JDKs. |
| Modpacks | Modrinth (`.mrpack`) and CurseForge (zip) packs; installing a pack creates its own instance with the right game version + loader. |
| Accounts | Microsoft (MSA → Xbox Live → XSTS → Minecraft), Ely.by (password or OAuth), sx.acc (SXMWARE account service), offline. Tokens never reach the frontend. |
| Multiplayer | LAN discovery (no infrastructure), global browser + share codes over a public MQTT broker or your own Redis, UDP hole punching with relay fallback, integrated dedicated server for hosting. |
| Extras | Discord Rich Presence, skin preview/upload, wallpapers & accent themes, TR/EN i18n, signed auto-updater. |

## Architecture

```
┌──────────────────────────── Tauri window (WebView) ────────────────────────────┐
│ React 19 + Vite + Tailwind 4 + Radix                                            │
│  pages/ ─ components/ ─ hooks/queries (TanStack Query) ─ stores/ (zustand)      │
│  services/*.ts  ── invoke() ──┐          ┌── events (progress, jobs, auth)       │
└───────────────────────────────┼──────────┼───────────────────────────────────────┘
                                ▼          │
┌──────────────────────────── Rust backend (src-tauri) ──────────────────────────┐
│ commands/   IPC surface (#[tauri::command])                                     │
│ auth/       msa · elyby · sxacc · offline · oauth loopback · OS credential vault│
│ instances/  installer · loaders · launch (JVM argv) · host_server               │
│ mods/       modrinth · curseforge · mrpack · resolver · downloader · java       │
│ network/    localnet (LAN) · mqtt/redis directory · holepunch · relay · bridge  │
│ store/      SQLite (rusqlite, WAL) · settings · migrations                      │
│ presence.rs Discord IPC · jobs.rs progress/cancel · config.rs paths + settings  │
└─────────────────────────────────────────────────────────────────────────────────┘
```

Design rules worth keeping:

* The frontend only ever sees `AccountSummary` objects. Access/refresh tokens
  live in the OS credential vault (Keychain / Credential Manager / Secret
  Service) and are assembled into the JVM command line inside Rust.
* Launch command lines are logged via `redacted_command_line()` only; Redis
  URLs are shown/logged via `redact_redis_url()`.
* No secret is compiled into the binary. Optional secrets are read from the
  environment at runtime or entered by the user in Settings.

## Folder structure

```
.
├── src/                    React app
│   ├── pages/              routed screens (dashboard, library, modpacks, servers, settings, skin, …)
│   ├── components/         UI by feature (account, instance, mods, servers, layout, ui primitives)
│   ├── hooks/              TanStack Query hooks + backend event hooks
│   ├── services/           typed IPC wrappers; mockBackend.ts powers `pnpm dev` in a plain browser
│   ├── stores/             zustand stores (ui, jobs, login, sessions, accent)
│   ├── i18n/               i18next setup + locales/en.json, locales/tr.json
│   ├── lib/                pure helpers (+ *.test.ts, run with vitest)
│   └── types/              shared TS types mirroring the Rust models
├── src-tauri/              Rust crate `sxmlauncher` (see Architecture)
│   ├── tauri.conf.json     window, CSP, bundle, deep links, updater endpoints + public key
│   └── capabilities/       Tauri 2 permission set for the main window
├── public/                 static assets (brand marks, loader icons/backgrounds)
├── scripts/                version sync, updater feed generator, icon generator, CI helper
├── docs/                   releasing.md, discord-rich-presence.md
└── .github/workflows/      build.yml (CI, Linux + Windows), release.yml (signed release)
```

## Environment / configuration

Copy the template and fill in only what you need:

```bash
cp .env.example .env
```

Every variable is optional and documented inline in
[`.env.example`](.env.example). The backend reads them **at runtime** (they are
not baked into the build). The app does not load `.env` on its own, so export
it into the shell that starts the app:

```bash
set -a; . ./.env; set +a
```

| Variable | Kind | Purpose |
|---|---|---|
| `SXML_CURSEFORGE_API_KEY` | secret | CurseForge Core API key (Modrinth works without a key). |
| `SXML_DISCORD_APPLICATION_ID` | public | Discord app id for Rich Presence; empty disables it. |
| `SXML_MSA_CLIENT_ID` | public | Override the Microsoft public client id (default `00000000402b5328`). |
| `SXML_ELYBY_CLIENT_ID` | public | Ely.by OAuth client (default `sxmlauncher3`, public desktop app). |
| `SXML_ELYBY_CLIENT_SECRET` / `SXML_ELYBY_REDIRECT_URI` | secret / config | Only for your own Ely.by *web* app. |
| `SXACC_BASE_URL` | config | sx.acc server origin; empty = provider unavailable. |
| `SXML_REDIS_URL` | secret if it has credentials | Optional Redis directory (e.g. Upstash `rediss://`). |
| `SXML_RELAY_URL`, `SXML_STUN_SERVERS` | config | P2P relay fallback and STUN servers. |

Values entered in Settings (CurseForge key, Ely.by secret, Redis URL, host
password) are currently stored in plain text (SQLite + `settings.json`) under the OS
app-data dir, not in the credential vault — see HANDOVER.md. Account tokens
*are* in the vault.

## Development

Prerequisites

* Node.js 22+ and pnpm 11 (`corepack enable`; version pinned in `package.json`)
* Rust stable ≥ 1.87 (`rustup`)
* **Linux**: `libwebkit2gtk-4.1-dev libgtk-3-dev libsoup-3.0-dev libayatana-appindicator3-dev librsvg2-dev libdbus-1-dev libssl-dev pkg-config build-essential`
* **Windows**: WebView2 runtime + Visual Studio Build Tools (C++ workload)

```bash
pnpm install --frozen-lockfile

pnpm dev          # UI only, in a browser, against services/mockBackend.ts
pnpm dev:app      # full desktop app (Vite + Tauri, hot reload)

pnpm typecheck    # tsc (app + node configs)
pnpm test         # vitest
pnpm build        # typecheck + production frontend bundle (dist/)
node scripts/preview-server.mjs   # serve dist/ to smoke-test the prod bundle

cd src-tauri
cargo check       # fast Rust check
cargo test --lib  # Rust unit tests (a few network/launch tests are #[ignore])
```

Local installers: `pnpm build:app` → `src-tauri/target/release/bundle/`.
Without `TAURI_SIGNING_PRIVATE_KEY` the updater artifact step fails; run
`node scripts/ci-disable-updater.mjs` first for an unsigned local build (do not
commit the resulting `tauri.conf.json` change).

## CI builds

* **`.github/workflows/build.yml`** — on pull requests, pushes to `main` /
  `handover/**`, or manual dispatch.
  * `build-linux` (ubuntu-22.04): typecheck, vitest, `cargo test --lib`, then an
    unsigned `.deb` + `.AppImage` uploaded as the `SXMLAUNCHER-linux` artifact.
  * `build-windows`: unsigned NSIS + MSI + portable exe uploaded as
    `SXMLAUNCHER-windows`.
  * Needs no secrets.
* **`.github/workflows/release.yml`** — manual (`workflow_dispatch` with an
  existing tag and the `LINUX-QA-PASSED` confirmation). Builds signed Windows
  installers with `secrets.TAURI_SIGNING_PRIVATE_KEY` /
  `secrets.TAURI_SIGNING_PRIVATE_KEY_PASSWORD`, generates `latest.json` and
  publishes a GitHub release. See [docs/releasing.md](docs/releasing.md).

## How the main subsystems work

### Microsoft sign-in (`src-tauri/src/auth/msa.rs`)
With the default public client `00000000402b5328` the browser opens the classic
"Sign in to Minecraft" page on `login.live.com`; the redirect comes back through
the `ms-xal-00000000402b5328://` deep link (registered by
`tauri-plugin-deep-link`). A custom Azure client id switches to Azure AD v2 +
PKCE with an `http://localhost` loopback listener. A device-code flow
(`microsoft.com/link`) is the fallback. Tokens are exchanged MSA → Xbox Live →
XSTS → `api.minecraftservices.com` and the refresh token goes to the vault.

### Ely.by (`auth/elyby.rs`)
Two paths: username/password through Ely.by's Yggdrasil authserver, or OAuth.
The public desktop client `sxmlauncher3` has no redirect URI, so its browser
flow is a device-code style page (`account.ely.by/code`). A custom web app uses
the loopback `http://localhost:25564/elyby/callback` and a client secret. Ely.by
sessions launch the game with **authlib-injector** (downloaded and sha256
verified) pointed at `authserver.ely.by`.

### sx.acc (`auth/sxacc.rs`)
SXMWARE's own account service. The launcher ships **no host**: the origin comes
from `SXACC_BASE_URL` or Settings. It uses `{BASE}/v1/auth/register|login|refresh`,
`{BASE}/v1/profile` (+ `/skin` upload), optional discovery at `GET {BASE}/v1`,
and the OAuth public client `sxmlauncher` with redirect
`sxmlauncher://auth/callback`. The game is launched with authlib-injector
pointed at `{BASE}/authlib/`.

### Token lifecycle (`auth/mod.rs`, `auth/vault.rs`)
`ensure_tokens` refreshes shortly before expiry and coalesces concurrent
refreshes per account (rotating refresh tokens). An `Unauthorized` refresh marks
the account signed-out but keeps the row. See HANDOVER.md for the planned
refresh hardening.

### P2P hosting / joining (`src-tauri/src/network/`)
* **LAN**: UDP broadcast beacons; zero setup.
* **Directory**: listings, share codes and signaling go over retained messages
  on a public MQTT broker (default `broker.emqx.io:8883`, TLS) — or a Redis you
  run (`SXML_REDIS_URL`). Listings expire automatically when a host stops
  heart-beating.
* **Connect**: STUN discovers the public mapping, both peers punch UDP, and a
  small stop-and-wait framed tunnel carries the Minecraft TCP stream. Symmetric
  NATs fall back to the WebSocket/TCP relay (`SXML_RELAY_URL`).
* **Bridge**: the guest gets a loopback port and the game is launched with
  `--server 127.0.0.1 --port <port>`; the
  host side bridges to the integrated server started by `instances/host_server.rs`.
* **Share codes**: `SXM1-…` base32 payload with CRC (`network/code.rs`).

### Modpacks (`src-tauri/src/mods/`)
`ModEngine` resolves a plan (resolver) → downloads in parallel with hash
verification (downloader, content-addressed cache) → extracts `.mrpack`
overrides / CurseForge `manifest.json` overrides. Installing a pack creates a new
instance, installs the matching loader (`instances/loaders.rs`) and records the
outcome; launch builds the JVM argv from the merged version JSON
(`instances/launch.rs`), substituting every Mojang placeholder explicitly.

## License

MIT — see [LICENSE](LICENSE).
