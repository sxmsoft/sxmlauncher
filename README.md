# SXMLauncher

Production Minecraft launcher desktop app: isolated instances, Microsoft & Ely.by auth (with skins), Modrinth/CurseForge modpacks, and one-click P2P hosting with join codes.

## Stack

- **Tauri 2** + Rust backend
- **React 19** + TypeScript + Vite frontend
- Tailwind CSS + Radix primitives
- pnpm (single package)

## Features

| Area | What works |
|------|------------|
| **Instances** | Create/manage isolated instances (name, MC version, vanilla/Fabric/Quilt/Forge/NeoForge, memory, Java). Mojang installs; launch argv is `java [jvm args] MainClass [game args]`. |
| **Auth** | Microsoft browser sign-in on the real “Sign in to Minecraft” page (`login.live.com`, public client `00000000402b5328`, `ms-xal-` callback) plus a device-code fallback on `microsoft.com/link`. A custom Azure client id still uses PKCE + `http://localhost`. Ely.by username/password, and browser sign-in: the public desktop client `sxmlauncher3` opens `https://account.ely.by/code` (it rejects every redirect URI); a web app with `SXML_ELYBY_CLIENT_SECRET` uses loopback `http://localhost:25564/elyby/callback`. Skins render in the UI. Ely.by launches download **authlib-injector** with sha256 verify. |
| **Modpacks** | Installing a Modrinth/CurseForge pack **creates its own instance** (game version + loader + mods). |
| **Play modes** | Clear UI split: **Singleplayer** (Play offline world) vs **Multiplayer/Host**. Host starts from the launcher: pick instance → Host → integrated dedicated server + port probe + join code / P2P on `0.0.0.0`. |
| **Jobs** | Progress events, cancelable downloads, clear errors. |

## Prerequisites (Windows focus)

- [Node.js 22+](https://nodejs.org/) and [pnpm 11+](https://pnpm.io/)
- [Rust stable](https://rustup.rs/) (1.87+)
- Windows: [WebView2](https://developer.microsoft.com/microsoft-edge/webview2/) (usually preinstalled)
- Visual Studio Build Tools with C++ workload (for Tauri)

## Setup

```bash
# Clone and enter the repo
cd sxmlauncher

# Copy env template and fill secrets (never commit real secrets)
cp .env.example .env

# Frontend deps
pnpm install

# Rust deps are fetched on first build
```

### Auth / API secrets (env)

| Variable | Purpose |
|----------|---------|
| `SXML_MSA_CLIENT_ID` | Optional. Default `00000000402b5328` uses the legacy Minecraft login page. A custom Azure app id switches to the v2 PKCE loopback and must allow `http://localhost` |
| `SXML_ELYBY_CLIENT_ID` | Default `sxmlauncher3` (public desktop client, no redirect) |
| `SXML_ELYBY_CLIENT_SECRET` | Only for your own Ely.by **web** application. Leave empty for `sxmlauncher3`. Never commit it |
| `SXML_ELYBY_REDIRECT_URI` | Used only with a web application secret. Must match registration; default `http://localhost:25564/elyby/callback` |
| `SXML_CURSEFORGE_API_KEY` | CurseForge Core API key |

## Develop

```bash
# Typecheck frontend
pnpm typecheck

# Vite-only UI (mock IPC when not in Tauri)
pnpm dev

# Full desktop app (Tauri + Vite)
pnpm dev:app

# Rust unit tests
cd src-tauri && cargo test
```

## Build

```bash
# Production frontend bundle
pnpm build

# Native installer. Windows NSIS/MSI publishing is paused in CI until
# Linux desktop QA passes. Local `tauri build` still works on any OS.
pnpm build:app
```

Artifacts land under `src-tauri/target/release/bundle/`. GitHub tag
releases do not publish Windows installers until the release workflow
gate `LINUX-QA-PASSED` is set.

## Architecture (short)

```
src/                 React UI (pages, components, IPC services)
src-tauri/src/
  auth/              Microsoft, Ely.by, offline, vault
  instances/         install, launch, integrated host server
  mods/              Modrinth, CurseForge, mrpack, authlib-injector
  network/           directory, hole-punch, bridge, join codes
  commands/          Tauri IPC surface
  store/             SQLite
```

## License

MIT — see [LICENSE](LICENSE).
