import { defineConfig } from "vite";

// Tauri expects a fixed dev port; the web source lives in frontend/, so set it as
// Vite's project root. `base: "./"` makes built asset URLs relative, which the
// Tauri custom protocol requires. Output goes to frontend/dist.
export default defineConfig({
  root: "frontend",
  base: "./",
  clearScreen: false,
  server: {
    port: 5173,
    strictPort: true,
    host: "127.0.0.1",
    watch: { ignored: ["**/src-tauri/**"] },
  },
  envPrefix: ["VITE_", "TAURI_"],
  build: {
    target: ["es2021", "chrome100", "safari13"],
    outDir: "dist",
    emptyOutDir: true,
    minify: "esbuild",
    sourcemap: false,
  },
});
