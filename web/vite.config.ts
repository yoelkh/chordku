import { svelte } from "@sveltejs/vite-plugin-svelte";
import { defineConfig } from "vitest/config";

const backend = process.env.CHORDKU_BACKEND ?? "http://127.0.0.1:7464";

export default defineConfig({
  plugins: [svelte()],
  build: { outDir: "dist", emptyOutDir: true, target: "es2022", assetsInlineLimit: 0 },
  server: { port: 5173, proxy: { "/api": { target: backend, changeOrigin: false } } },
  test: { include: ["src/**/*.test.ts"], environment: "node" },
});
