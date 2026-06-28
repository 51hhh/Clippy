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
        settings: resolve(__dirname, "settings.html"),
        pin: resolve(__dirname, "pin.html"),
        capture: resolve(__dirname, "capture.html"),
      },
    },
  },
  server: {
    port: 1420,
    strictPort: true,
  },
  test: {
    environment: "jsdom",
    include: ["tests/**/*.test.js"],
    globals: false,
  },
});
