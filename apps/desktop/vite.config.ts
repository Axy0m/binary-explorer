import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

// Tauri expects a fixed dev server port and does not want Vite to clear the
// terminal (Tauri prints its own diagnostics there).
export default defineConfig({
  plugins: [react()],
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
  },
  // Produce output the Tauri config points at (frontendDist: "../dist").
  build: {
    target: "es2021",
    outDir: "dist",
    emptyOutDir: true,
  },
});
