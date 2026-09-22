#!/usr/bin/env node
/**
 * Builds `latest.json`, the signed-release feed the Tauri updater reads.
 *
 * Point it at a folder containing the built installers (and their `.sig`
 * files, produced automatically when `TAURI_SIGNING_PRIVATE_KEY` is set) and
 * upload the emitted `latest.json` next to those installers. The feed format
 * is the one `tauri.conf.json → plugins.updater.endpoints` expects.
 *
 * Usage:
 *   node scripts/generate-latest-json.mjs <bundle-dir> <version> [notes]
 *       [--base-url <url>] [--out <path>]
 *
 * The download URLs in the feed are relative (`download/<dir>/<file>`) by
 * default, which works when `latest.json` and the installers are served from
 * the same directory. Pass `--base-url` (or set `UPDATE_FEED_BASE_URL`) to
 * emit absolute URLs — the GitHub release workflow uses this to point the
 * feed at the release's `download/<tag>/…` asset URLs.
 *
 * Examples:
 *   node scripts/generate-latest-json.mjs src-tauri/target/release/bundle 0.2.0 "Bugfixes"
 *   node scripts/generate-latest-json.mjs bundle 0.2.0 \
 *     --base-url "https://github.com/sxmlauncher/sxmlauncher/releases/download/v0.2.0"
 */
import { readFile, readdir, writeFile } from "node:fs/promises";
import { join, resolve, basename } from "node:path";

const args = process.argv.slice(2);
const positional = [];
let baseUrl = process.env.UPDATE_FEED_BASE_URL ?? "";
let outPath;

for (let i = 0; i < args.length; i++) {
  const arg = args[i];
  if (arg === "--base-url") baseUrl = args[++i] ?? "";
  else if (arg === "--out") outPath = args[++i];
  else positional.push(arg);
}

const [bundleDirArg, versionArg, notesArg = ""] = positional;
if (!bundleDirArg || !versionArg) {
  console.error(
    "usage: node scripts/generate-latest-json.mjs <bundle-dir> <version> [notes] [--base-url <url>] [--out <path>]",
  );
  process.exit(1);
}

const bundleDir = resolve(bundleDirArg);
const version = versionArg.replace(/^v/, "");

/** platforms keyed by the updater's `{target}-{arch}` triples. */
const PLATFORMS = [
  { key: "windows-x86_64", dir: "nsis", ext: "-setup.exe" },
  { key: "linux-x86_64", dir: "appimage", ext: ".AppImage" },
  { key: "darwin-x86_64", dir: "macos", ext: ".app.tar.gz" },
  { key: "darwin-aarch64", dir: "macos", ext: ".app.tar.gz" },
];

const entries = await readdir(bundleDir, { withFileTypes: true });
const platforms = {};
const skipped = [];

for (const platform of PLATFORMS) {
  const dir = entries.find((entry) => entry.isDirectory() && entry.name === platform.dir);
  if (!dir) continue;

  const files = await readdir(join(bundleDir, platform.dir));
  const artifact = files.find(
    (file) =>
      file.includes(version) &&
      file.endsWith(platform.ext) &&
      !file.endsWith(".sig"),
  );
  if (!artifact) continue;

  const signature = files.find((file) => file === `${artifact}.sig`);
  if (!signature) {
    skipped.push(`${platform.key}: ${artifact} has no .sig — build with TAURI_SIGNING_PRIVATE_KEY set`);
    continue;
  }

  const sig = await readFile(join(bundleDir, platform.dir, signature), "utf8");
  // Relative URLs resolve against latest.json's own location on the server.
  const url = baseUrl
    ? `${baseUrl.replace(/\/+$/, "")}/${platform.dir}/${artifact}`
    : `download/${platform.dir}/${artifact}`;

  platforms[platform.key] = {
    signature: sig.trim(),
    url,
  };
}

for (const message of skipped) console.warn(`! ${message}`);

if (Object.keys(platforms).length === 0) {
  console.error(`no signed artifacts for version ${version} under ${bundleDir}`);
  process.exit(1);
}

const feed = {
  version,
  notes: notesArg,
  pub_date: new Date().toISOString(),
  platforms,
};

const out = outPath ? resolve(outPath) : join(bundleDir, "latest.json");
await writeFile(out, JSON.stringify(feed, null, 2) + "\n");
console.log(`wrote ${out}`);
console.log(`  version:  ${feed.version}`);
for (const [key, value] of Object.entries(platforms)) {
  console.log(`  ${key.padEnd(16)} ${value.url}`);
}
console.log("\nUpload latest.json together with the installers it points at, then tag the release.");
