import { defineConfig } from 'vite'
import vue from '@vitejs/plugin-vue'
import path from "path";
import fs from "fs";

// Hunsterverse (1.5.4-hv3): the header's "release ver." shows the launcher's own
// version (tauri.conf.json, the one the self-updater compares), not a typed copy.
const appVersion = JSON.parse(
  fs.readFileSync(new URL("./src-tauri/tauri.conf.json", import.meta.url), "utf8")
).package.version;

export default defineConfig({
  // prevent vite from obscuring rust errors
  clearScreen: false,
  // Tauri expects a fixed port, fail if that port is not available
  server: {
    strictPort: true,
  },
  plugins: [vue()],
  define: {
    __HV_APP_VERSION__: JSON.stringify(appVersion),
  },
  // to access the Tauri environment variables set by the CLI with information about the current target
  envPrefix: ['VITE_', 'TAURI_PLATFORM', 'TAURI_ARCH', 'TAURI_FAMILY', 'TAURI_PLATFORM_VERSION', 'TAURI_PLATFORM_TYPE', 'TAURI_DEBUG'],
  build: {
    // Tauri uses Chromium on Windows and WebKit on macOS and Linux
    target: process.env.TAURI_PLATFORM == 'windows' ? 'chrome105' : 'safari13',
    // don't minify for debug builds
    minify: !process.env.TAURI_DEBUG ? 'esbuild' : false,
    // produce sourcemaps for debug builds
    sourcemap: !!process.env.TAURI_DEBUG,
    // Alt-client assets are intentionally large enough to exceed Vite's default warning threshold.
    chunkSizeWarningLimit: 4000,
  },
})
