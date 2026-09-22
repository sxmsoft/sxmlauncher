#!/usr/bin/env node
/**
 * Syncs a version into every manifest the release pipeline reads:
 * package.json, src-tauri/tauri.conf.json, src-tauri/Cargo.toml and
 * src-tauri/Cargo.lock.
 *
 * The release workflow runs this with the pushed tag (`v0.2.0` → `0.2.0`)
 * before `tauri build`, so the installer names and updater feed always match
 * the tag without manual edits.
 *
 * Usage:
 *   node scripts/sync-version.mjs <version>   # e.g. 0.2.0 or v0.2.0
 *   node scripts/sync-version.mjs v0.2.0 --check   # CI guard: exit 1 if any
 *                                                  # manifest already differs
 */
import { readFile, writeFile } from "node:fs/promises";

const args = process.argv.slice(2);
const check = args.includes("--check");
const versionArg = args.find((arg) => arg !== "--check");
if (!versionArg) {
  console.error("usage: node scripts/sync-version.mjs <version> [--check]");
  process.exit(1);
}
const version = versionArg.replace(/^v/, "");
if (!/^\d+\.\d+\.\d+([-+].+)?$/.test(version)) {
  console.error(`not a valid semver version: ${version}`);
  process.exit(1);
}

/** Write keeping the file's original line endings (repos may use CRLF). */
async function writeText(path, text) {
  const eol = text.includes("\r\n") ? "\r\n" : "\n";
  await writeFile(path, text.replace(/\r?\n/g, eol));
  edits.push(`${path} → ${version}`);
}

const edits = [];

// package.json — JSON-aware so the rest of the file keeps its formatting.
{
  const path = "package.json";
  const text = await readFile(path, "utf8");
  const json = JSON.parse(text);
  if (json.version !== version) {
    if (check) die(`${path}: version is ${json.version}, expected ${version}`);
    json.version = version;
    await writeText(path, JSON.stringify(json, null, 2) + "\n");
  }
}

// src-tauri/tauri.conf.json — same JSON treatment.
{
  const path = "src-tauri/tauri.conf.json";
  const text = await readFile(path, "utf8");
  const json = JSON.parse(text);
  if (json.version !== version) {
    if (check) die(`${path}: version is ${json.version}, expected ${version}`);
    json.version = version;
    await writeText(path, JSON.stringify(json, null, 2) + "\n");
  }
}

// src-tauri/Cargo.toml — the [package] version = "…" field (first match only;
// workspace-dependency or other `version =` lines live outside [package]).
{
  const path = "src-tauri/Cargo.toml";
  const text = await readFile(path, "utf8");
  const re = /^(\[package\][^[]*?^version\s*=\s*)"([^"]*)"/ms;
  const match = text.match(re);
  if (!match) {
    if (check) die(`${path}: [package].version field not found`);
  } else if (match[2] !== version) {
    if (check) die(`${path}: version is ${match[2]}, expected ${version}`);
    await writeFile(path, text.replace(re, `$1"${version}"`));
    edits.push(`${path} → ${version}`);
  }
}

// src-tauri/Cargo.lock — only the sxmlauncher [[package]] entry. Editing the
// field directly avoids a full `cargo update` (which would bump unrelated
// dependency versions and bloat the release diff). Lock entries look like:
//   [[package]]
//   name = "sxmlauncher"
//   version = "0.1.0"
{
  const path = "src-tauri/Cargo.lock";
  const text = await readFile(path, "utf8");
  const entryRe = /\[\[package\]\]\r?\nname = "sxmlauncher"\r?\nversion = "([^"]*)"/;
  const match = text.match(entryRe);
  if (!match) {
    if (check) die(`${path}: no [[package]] entry for sxmlauncher`);
  } else if (match[1] !== version) {
    if (check) die(`${path}: sxmlauncher version is ${match[1]}, expected ${version}`);
    await writeFile(path, text.replace(entryRe, (entry) => entry.replace(/version = "[^"]*"/, `version = "${version}"`)));
    edits.push(`${path} → ${version}`);
  }
}

if (edits.length === 0) {
  console.log(`all manifests already at ${version}`);
} else {
  for (const edit of edits) console.log(`updated ${edit}`);
}

function die(message) {
  console.error(`version mismatch: ${message}`);
  console.error("run `node scripts/sync-version.mjs <version>` before building");
  process.exit(1);
}
