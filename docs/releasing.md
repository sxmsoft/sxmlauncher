# Releasing SXMLauncher

> **Current state:** `.github/workflows/release.yml` runs only on manual
> `workflow_dispatch` (existing tag + typing `LINUX-QA-PASSED`); the tag-push
> trigger is disabled. After pushing the tag, start the workflow from the
> Actions tab. Everything below still applies once the tag trigger is
> re-enabled.

The release pipeline builds the signed Windows installers, generates the
updater feed and publishes everything to a GitHub release. Installed launchers
then find the new version through the updater endpoint baked into the app.

## One-time repository setup

1. **Add the signing secret.** Repository → Settings → Secrets and variables →
   Actions → New repository secret:

   | Secret | Value |
   |---|---|
   | `TAURI_SIGNING_PRIVATE_KEY` | Full contents of `.keys/sxmlauncher.key` (the file, not a path) |
   | `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` | Only if the keypair was generated with a password |

   The keypair lives in the gitignored `.keys/` directory. **Back it up
   somewhere safe** — losing it means you can no longer ship updates that
   installed launchers will accept; every user would have to reinstall
   manually.

2. **Match the feed URL to the real repository.** `src-tauri/tauri.conf.json`
   lists `https://github.com/sxmlauncher/sxmlauncher/releases/latest/download/latest.json`
   as the primary updater endpoint. If the release repository is different,
   change it to `https://github.com/<owner>/<repo>/releases/latest/download/latest.json`
   *before* the first tagged release — the URL is compiled into the binary.

## Cutting a release

```bash
# 1. Bump versions everywhere (package.json, tauri.conf.json, Cargo.toml/lock)
node scripts/sync-version.mjs v0.2.0
git add package.json src-tauri/tauri.conf.json src-tauri/Cargo.toml src-tauri/Cargo.lock
git commit -m "chore(release): v0.2.0"

# 2. Tag and push — this triggers the release workflow
git tag v0.2.0
git push origin main v0.2.0
```

The workflow (`.github/workflows/release.yml`) then:

1. Runs the test suite, typecheck and a frontend build as a release gate.
2. Builds **NSIS** (`SXMLauncher_0.2.0_x64-setup.exe`) and **MSI**
   (`SXMLauncher_0.2.0_x64_en-US.msi`) installers, signed with the
   `TAURI_SIGNING_PRIVATE_KEY` secret — each installer gets a `.sig` file.
3. Runs `scripts/generate-latest-json.mjs` to produce `latest.json`, the feed
   the Tauri updater consumes, with absolute download URLs pointing at the
   release assets.
4. Verifies the feed (version match, signatures present, asset URLs reachable).
5. Publishes installers, signatures and `latest.json` to the GitHub release.

A failed gate aborts the release; nothing is published.

## Re-running / fixing a release

The workflow has a **Run workflow** button (`workflow_dispatch`) that takes an
existing tag (e.g. `v0.2.0`) and rebuilds + re-uploads its assets. Note that
GitHub serves `releases/latest/download/latest.json` with caches — after
re-publishing assets for a tag, published launchers may take a few minutes to
see the corrected feed.

## Local dry run

You can exercise the same steps locally:

```bash
pnpm install
pnpm typecheck && cargo test --lib --release --manifest-path src-tauri/Cargo.toml
TAURI_SIGNING_PRIVATE_KEY="$(cat .keys/sxmlauncher.key)" pnpm tauri build -b msi,nsis
node scripts/generate-latest-json.mjs src-tauri/target/release/bundle v0.2.0 "Notes" \
  --base-url "https://github.com/<owner>/<repo>/releases/download/v0.2.0"
```

The feed lands at `src-tauri/target/release/bundle/latest.json`; upload it
alongside the installers for a manual release.

## How updates reach users

1. Launcher starts → Settings → **Updates** (or the automatic check) hits the
   first endpoint; on failure it falls back to `updates.sxmlauncher.dev`.
2. The updater compares the feed version against the running version.
3. On a newer version it downloads the artifact for `windows-x86_64` and
   verifies the minisign signature against the embedded public key — a bad or
   missing signature aborts the install.
4. "Restart & install" runs the downloaded installer and relaunches.
