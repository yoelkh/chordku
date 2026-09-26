// Folds the viewer build (dist-viewer/) into one self-contained dist/viewer.html: script and
// stylesheet inlined, so the file works from disk (file://), WhatsApp, and GitHub Pages alike.
import { readFileSync, writeFileSync, rmSync, existsSync } from "node:fs";
import { join } from "node:path";

const src = "dist-viewer";
const html = readFileSync(join(src, "viewer.html"), "utf8");
let out = html;

out = out.replace(/<script type="module"[^>]*src="\/?([^"]+)"><\/script>/g, (_, file) => {
  const js = readFileSync(join(src, file), "utf8").replace(/<\/script/gi, "<\\/script");
  return `<script type="module">${js}</script>`;
});
out = out.replace(/<link rel="stylesheet"[^>]*href="\/?([^"]+)">/g, (_, file) => {
  const css = readFileSync(join(src, file), "utf8").replace(/<\/style/gi, "<\\/style");
  return `<style>${css}</style>`;
});
if (!out.includes("<!--CHORDKU_SETLIST-->")) throw new Error("viewer.html lost its data placeholder");
if (/src="\/assets\//.test(out) || /href="\/assets\//.test(out)) throw new Error("viewer.html still references external assets");

writeFileSync(join("dist", "viewer.html"), out);
if (existsSync(src)) rmSync(src, { recursive: true });
console.log(`dist/viewer.html: ${(out.length / 1024).toFixed(0)} KB, self-contained`);
