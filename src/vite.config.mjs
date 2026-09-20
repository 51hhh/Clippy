import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import { dirname, resolve } from "path";
import { fileURLToPath } from "url";

const __dirname = dirname(fileURLToPath(import.meta.url));

export default defineConfig({
  root: ".",
  plugins: [react()],
  build: {
    outDir: "../dist",
    emptyOutDir: true,
    modulePreload: false,
    rollupOptions: {
      input: {
        main: resolve(__dirname, "index.html"),
        launcher: resolve(__dirname, "launcher.html"),
        settings: resolve(__dirname, "settings.html"),
        pin: resolve(__dirname, "pin.html"),
        viewer: resolve(__dirname, "viewer.html"),
        captureOverlay: resolve(__dirname, "capture-overlay.html"),
        longshotController: resolve(__dirname, "longshot-controller.html"),
        longshotGuide: resolve(__dirname, "longshot-guide.html"),
        recordingControl: resolve(__dirname, "recording-control.html"),
        recordings: resolve(__dirname, "recordings.html"),
        pinWorkspaces: resolve(__dirname, "pin-workspaces.html"),
      },
    },
  },
  server: {
    port: 1420,
    strictPort: true,
  },
  test: {
    environment: "jsdom",
    include: ["tests/**/*.test.js", "tests/**/*.test.ts", "tests/**/*.test.tsx"],
    globals: false,
  },
});
