// Unsigned CI builds cannot produce updater artifacts (they need
// TAURI_SIGNING_PRIVATE_KEY). Turn the updater bundle off for this checkout
// only; the committed tauri.conf.json is unchanged.
import { readFileSync, writeFileSync } from "node:fs";

const path = "src-tauri/tauri.conf.json";
const config = JSON.parse(readFileSync(path, "utf8"));
if (config.bundle) config.bundle.createUpdaterArtifacts = false;
if (config.plugins?.updater) config.plugins.updater.active = false;
writeFileSync(path, `${JSON.stringify(config, null, 2)}\n`);
console.log("updater artifacts disabled for this CI build");
