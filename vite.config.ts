import { resolve } from "node:path";
import { defineConfig } from "vite";
import { svelte } from "@sveltejs/vite-plugin-svelte";

// Tauri drives the dev server; it must be fixed-port and must not clear the screen.
const host = process.env.TAURI_DEV_HOST;

export default defineConfig({
  plugins: [svelte()],
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    host: host || false,
    hmr: host ? { protocol: "ws", host, port: 1421 } : undefined,
    watch: { ignored: ["**/src-tauri/**"] },
  },
  envPrefix: ["VITE_", "TAURI_ENV_*"],
  build: {
    // Two windows, two entry points: the capsule and the settings panel are
    // separate documents so the 428px island never loads the settings UI.
    rollupOptions: {
      input: {
        main: resolve(__dirname, "index.html"),
        settings: resolve(__dirname, "settings.html"),
      },
    },
    // WebView2 on Win10 1809+ is evergreen Chromium; target modern output.
    target: "chrome110",
    // Vite 8 minifies with oxc; naming esbuild here would pull in a dependency
    // that is no longer bundled.
    minify: true,
    sourcemap: false,
    cssMinify: "lightningcss",
    reportCompressedSize: false,
  },
});
