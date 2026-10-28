import { readFileSync } from "node:fs";
import react from "@vitejs/plugin-react";
import { defineConfig } from "vitest/config";

// The version shown in the app ("O aplikaci", footer) is the one in package.json.
const { version } = JSON.parse(readFileSync(new URL("./package.json", import.meta.url), "utf8")) as { version: string };

// Tauri expects a fixed dev port and fails when it is taken, hence strictPort.
const host = process.env.TAURI_DEV_HOST;

export default defineConfig({
  plugins: [react()],
  define: { __APP_VERSION__: JSON.stringify(version) },
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    host: host || false,
    hmr: host ? { protocol: "ws", host, port: 1421 } : undefined,
    watch: { ignored: ["**/src-tauri/**", "**/crates/**", "**/target/**"] },
  },
  envPrefix: ["VITE_", "TAURI_ENV_"],
  build: {
    // WebView2 on Windows 11 is evergreen Chromium.
    target: ["es2022", "chrome110"],
    outDir: "dist",
    emptyOutDir: true,
    sourcemap: !!process.env.TAURI_ENV_DEBUG,
  },
  test: {
    environment: "jsdom",
    include: ["src/**/*.test.{ts,tsx}", "feedback/src/**/*.test.ts", "scripts/**/*.test.mjs"],
    setupFiles: ["src/test/setup.ts"],
    env: { TZ: "Europe/Prague" },
  },
});
