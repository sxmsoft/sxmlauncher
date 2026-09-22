#!/usr/bin/env node
/**
 * Serves `dist/` over HTTP with SPA fallback so the *production* bundle can be
 * smoke-tested in a browser (dev mode hides prod-only React failures such as
 * unstable-store-selector loops).
 *
 * Usage: node scripts/preview-server.mjs [port]   (default 4173)
 */
import http from "node:http";
import { readFile } from "node:fs/promises";
import { extname, join, normalize } from "node:path";
import { fileURLToPath } from "node:url";

const port = Number(process.argv[2] ?? 4173);
const root = join(fileURLToPath(new URL(".", import.meta.url)), "..", "dist");

const MIME = {
  ".html": "text/html",
  ".js": "text/javascript",
  ".css": "text/css",
  ".png": "image/png",
  ".svg": "image/svg+xml",
  ".ico": "image/x-icon",
  ".woff2": "font/woff2",
};

http
  .createServer(async (req, res) => {
    try {
      const pathname = decodeURIComponent(new URL(req.url, "http://localhost").pathname);
      const candidate = join(root, normalize(pathname));
      // Anything without a file extension falls back to index.html (SPA routes).
      const file = extname(candidate) === "" ? join(root, "index.html") : candidate;
      const data = await readFile(file);
      res.writeHead(200, { "Content-Type": MIME[extname(file)] ?? "application/octet-stream" });
      res.end(data);
    } catch {
      res.writeHead(404);
      res.end();
    }
  })
  .listen(port, "127.0.0.1", () => console.log(`serving dist on http://127.0.0.1:${port}`));
