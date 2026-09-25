import { defineConfig } from "vite";
import { svelte } from "@sveltejs/vite-plugin-svelte";

// Tauri drives this dev server; keep the port fixed so tauri.conf.json can
// point at it, and never clear the screen so Rust build output stays visible.
export default defineConfig({
  plugins: [svelte()],
  root: "ui",
  clearScreen: false,
  build: {
    outDir: "../dist",
    emptyOutDir: true,
    target: "esnext",
  },
  server: {
    port: 5183,
    strictPort: true,
  },
});
