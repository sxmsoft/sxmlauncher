import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";
import { fileURLToPath, URL } from "node:url";

// Tauri injects these so the dev server can be reached from the native window.
const host = process.env.TAURI_DEV_HOST;

// https://vite.dev/config/
export default defineConfig({
  plugins: [react(), tailwindcss()],

  resolve: {
    alias: {
      "@": fileURLToPath(new URL("./src", import.meta.url)),
    },
  },

  // Tauri expects a fixed port and fails if it is not available.
  server: {
    port: 1420,
    strictPort: true,
    host: host || false,
    hmr: host ? { protocol: "ws", host, port: 1421 } : undefined,
    watch: {
      // src-tauri is watched by the Rust side, not Vite.
      ignored: ["**/src-tauri/**"],
    },
  },

  clearScreen: false,
  envPrefix: ["VITE_", "TAURI_ENV_"],
  build: {
    // Tauri v2 ships a modern webview on every platform.
    target: "es2022",
    // Rolldown (Vite 8's bundler) minifies with its built-in Oxc minifier, so
    // this needs no separate toolchain. Debug builds keep readable output.
    minify: !process.env.TAURI_ENV_DEBUG,
    sourcemap: !!process.env.TAURI_ENV_DEBUG,
  },
});
