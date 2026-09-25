import { defineConfig, type Plugin } from "vite";
import { svelte } from "@sveltejs/vite-plugin-svelte";
import { cpSync, createReadStream, existsSync, statSync } from "node:fs";
import { join, resolve, sep } from "node:path";

/** Excalidraw's fonts, served at /excalidraw/fonts (see ui/src/lib/excalidraw.ts). */
const EXCALIDRAW_FONTS = resolve("node_modules/@excalidraw/excalidraw/dist/prod/fonts");

function excalidrawFonts(): Plugin {
  return {
    name: "excalidraw-fonts",
    configureServer(server) {
      server.middlewares.use("/excalidraw/fonts", (req, res, next) => {
        const rel = decodeURIComponent((req.url ?? "").split("?")[0]);
        const file = join(EXCALIDRAW_FONTS, rel);
        if (!file.startsWith(EXCALIDRAW_FONTS + sep) || !existsSync(file) || !statSync(file).isFile()) return next();
        res.setHeader("Content-Type", file.endsWith(".woff2") ? "font/woff2" : "application/octet-stream");
        createReadStream(file).pipe(res);
      });
    },
    closeBundle() {
      cpSync(EXCALIDRAW_FONTS, resolve("dist/excalidraw/fonts"), { recursive: true });
    },
  };
}

// Tauri drives this dev server; keep the port fixed so tauri.conf.json can
// point at it, and never clear the screen so Rust build output stays visible.
export default defineConfig({
  plugins: [svelte(), excalidrawFonts()],
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
