// The public pages as static templates (src/public, issue #522). Runs in
// web/ after `vite build` (npm run build):
//   1. builds src/public/public.css into dist/assets/public-[hash].css;
//   2. renders src/public/render.tsx through Vite's module runner;
//   3. writes dist/public-pages/templates/{de,en}/*.html.
// The web container fills the templates in at start.
/* global console, URL */
import { mkdir, readFile, rm, writeFile } from "node:fs/promises";
import { dirname, join } from "node:path";
import { build, runnerImport } from "vite";

const web = new URL("../..", import.meta.url).pathname;
const dist = join(web, "dist");
const templates = join(dist, "public-pages", "templates");
const manifestPath = join(dist, ".public-pages-manifest.json");

// 1. The stylesheet, hashed like the app's assets (immutable in nginx).
await build({
  configFile: false,
  root: web,
  logLevel: "warn",
  build: {
    outDir: dist,
    emptyOutDir: false,
    copyPublicDir: false,
    manifest: ".public-pages-manifest.json",
    rolldownOptions: {
      input: { public: join(web, "src/public/public.css") },
    },
  },
});
const manifest = JSON.parse(await readFile(manifestPath, "utf8"));
await rm(manifestPath);
const entry = Object.values(manifest).find(
  (chunk) => chunk.isEntry && chunk.name === "public",
);
const css = entry?.file.endsWith(".css") ? entry.file : entry?.css?.[0];
if (!css) throw new Error("public pages: public.css was not built");

// 2. The pages, with the app's own TSX and string catalogs.
const { module } = await runnerImport(join(web, "src/public/render.tsx"), {
  configFile: false,
  root: web,
  logLevel: "warn",
});
const pages = module.renderPublicTemplates({ stylesheet: `/${css}` });

// 3. One file per page and language.
await rm(templates, { recursive: true, force: true });
for (const { file, html } of pages) {
  const path = join(templates, file);
  await mkdir(dirname(path), { recursive: true });
  await writeFile(path, html);
}
console.log(`public pages: ${pages.length} templates, /${css}`);
