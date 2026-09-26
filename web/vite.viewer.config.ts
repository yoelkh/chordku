// The standalone setlist viewer: a single entry with everything bundled into one JS and one
// CSS file, which `scripts/inline-viewer.mjs` then folds into `dist/viewer.html`.
import { svelte } from "@sveltejs/vite-plugin-svelte";
import { defineConfig } from "vite";

export default defineConfig({
  plugins: [svelte()],
  build: {
    outDir: "dist-viewer",
    emptyOutDir: true,
    target: "es2022",
    assetsInlineLimit: 1e9,
    modulePreload: false,
    cssCodeSplit: false,
    rollupOptions: { input: "viewer.html", output: { inlineDynamicImports: true } },
  },
});
