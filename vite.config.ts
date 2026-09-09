// SOT: vite-config, dev-server-port, tauri-frontend-build
import { readFileSync } from "node:fs";
import { fileURLToPath, URL } from "node:url";
import { defineConfig, type Plugin } from "vite";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";

// WHAT:  Vite config for the Tauri webview frontend.
// WHY:   Tauri expects a fixed dev port (1420) and a static dist folder.
// HOW:   tauri.conf.json `build.devUrl` / `frontendDist` point here.
// WHERE: src-tauri/tauri.conf.json
// WHAT:  Serves the webview with Tauri's IPC bridge stubbed, so the UI can be
//        opened in an ordinary browser.
// WHY:   Building the desktop shell needs the platform's webkit2gtk headers; a
//        UI review should not be gated on having them. Enabled only by
//        `pnpm preview:ui`, and only on a dev server — `apply: "serve"` plus the
//        env check keep it out of every shipped bundle.
// WHERE: scripts/preview-bridge.js
function uiPreview(): Plugin {
  return {
    name: "db-free:ui-preview",
    apply: "serve",
    transformIndexHtml(html) {
      if (process.env["DBFREE_PREVIEW"] !== "1") return html;
      const bridge = readFileSync(fileURLToPath(new URL("./scripts/preview-bridge.js", import.meta.url)), "utf8");
      // Prepended to <head> so it defines the bridge before any app module runs.
      return html.replace("<head>", `<head><script>${bridge}</script>`);
    },
  };
}

export default defineConfig({
  plugins: [react(), tailwindcss(), uiPreview()],
  // Mirrors tsconfig `paths`: Vite resolves `@/…` itself, tsc only type-checks it.
  resolve: { alias: { "@": fileURLToPath(new URL("./src", import.meta.url)) } },
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    watch: { ignored: ["**/src-tauri/**"] },
  },
  build: {
    target: ["es2022", "safari16"],
    sourcemap: false,
  },
});
