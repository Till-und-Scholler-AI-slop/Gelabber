// Every field state (idle and in keyboard focus) in four themes, built
// from components/ui and compared with the screenshots in
// scripts/field-gallery/baseline (#561). It catches what the CSS tests
// cannot see: a second frame, a gap, a box inside a box.
//
// Run in web/: node scripts/field-gallery.mjs
// After a deliberate change: UPDATE_FIELD_GALLERY=1 node scripts/field-gallery.mjs
// and look at the new baseline pictures before committing them.
// A mismatch writes the actual and a diff picture to field-gallery-out/.
// GELABBER_FIELD_BROWSER names a Chromium other than Playwright's.
/* global process, console, document, Image, Buffer, URL */
import {
  mkdir,
  mkdtemp,
  readFile,
  readdir,
  rm,
  writeFile,
} from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { chromium } from "playwright";
import { build } from "vite";

const web = fileURLToPath(new URL("..", import.meta.url));
const BASELINE = join(web, "scripts/field-gallery/baseline");
const OUT = join(web, "field-gallery-out");
const PAGE = "scripts/field-gallery/index.html";
const ORIGIN = "https://fields.test";
const UPDATE = Boolean(process.env.UPDATE_FIELD_GALLERY);
/** A pixel differs when a channel moves by more than this. */
const CHANNEL = 64;
/** Up to this share of a picture may differ: text antialiasing between
 * machines and browser builds, not a frame (a lost 1px ring around a
 * field is about 4%). */
const SHARE = 0.02;

const out = await mkdtemp(join(tmpdir(), "field-gallery-"));
let browser;
try {
  await build({
    root: web,
    logLevel: "error",
    build: {
      outDir: out,
      emptyOutDir: true,
      rollupOptions: { input: [join(web, PAGE)] },
    },
  });
  const files = new Set(
    (await readdir(out, { recursive: true })).map((name) => `/${name}`),
  );
  browser = await chromium.launch({
    executablePath: process.env.GELABBER_FIELD_BROWSER || undefined,
    headless: true,
  });
  const page = await browser.newPage({
    viewport: { width: 1200, height: 900 },
    deviceScaleFactor: 1,
    colorScheme: "dark",
    reducedMotion: "reduce",
  });
  await page.route("**/*", async (route) => {
    const url = new URL(route.request().url());
    if (url.origin !== ORIGIN) return route.abort();
    const path = url.pathname === "/" ? `/${PAGE}` : url.pathname;
    if (!files.has(path)) return route.fulfill({ status: 404, body: "" });
    return route.fulfill({ path: join(out, path) });
  });
  await page.goto(`${ORIGIN}/`);
  await page.waitForSelector("[data-field]");
  await page.evaluate(() => document.fonts.ready);

  const names = await page.$$eval("[data-field]", (cells) =>
    cells.map((cell) => cell.getAttribute("data-field")),
  );
  const shots = [];
  for (const name of names) {
    const cell = page.locator(`[data-field="${name}"]`);
    await page.evaluate(() => document.activeElement?.blur());
    shots.push([`${name}`, await cell.screenshot({ animations: "disabled" })]);
    const target = cell.locator("input, select, textarea").first();
    if (await target.isDisabled()) continue;
    // Keyboard focus, so :focus-visible holds for every kind of field.
    await target.focus();
    await page.keyboard.press("Shift+Tab");
    await page.keyboard.press("Tab");
    shots.push([
      `${name}-focus`,
      await cell.screenshot({ animations: "disabled", caret: "hide" }),
    ]);
  }

  if (UPDATE) {
    await rm(BASELINE, { recursive: true, force: true });
    await mkdir(BASELINE, { recursive: true });
    for (const [name, png] of shots)
      await writeFile(join(BASELINE, `${name}.png`), png);
    console.log(`field gallery: wrote ${shots.length} baseline pictures`);
  } else {
    const failed = [];
    for (const [name, png] of shots) {
      let want;
      try {
        want = await readFile(join(BASELINE, `${name}.png`));
      } catch {
        failed.push(`${name}: no baseline`);
        continue;
      }
      const result = await page.evaluate(
        async ([a, b, channel]) => {
          const load = (data) =>
            new Promise((resolve, reject) => {
              const image = new Image();
              image.onload = () => resolve(image);
              image.onerror = reject;
              image.src = `data:image/png;base64,${data}`;
            });
          const [x, y] = await Promise.all([load(a), load(b)]);
          if (x.width !== y.width || x.height !== y.height)
            return { size: `${x.width}x${x.height} vs ${y.width}x${y.height}` };
          const pixels = (image) => {
            const canvas = document.createElement("canvas");
            canvas.width = image.width;
            canvas.height = image.height;
            const context = canvas.getContext("2d");
            context.drawImage(image, 0, 0);
            return context.getImageData(0, 0, image.width, image.height);
          };
          const p = pixels(x).data;
          const q = pixels(y);
          let differing = 0;
          for (let i = 0; i < p.length; i += 4) {
            const off =
              Math.abs(p[i] - q.data[i]) > channel ||
              Math.abs(p[i + 1] - q.data[i + 1]) > channel ||
              Math.abs(p[i + 2] - q.data[i + 2]) > channel;
            if (off) {
              differing++;
              q.data.set([255, 0, 255, 255], i);
            }
          }
          const canvas = document.createElement("canvas");
          canvas.width = q.width;
          canvas.height = q.height;
          canvas.getContext("2d").putImageData(q, 0, 0);
          return {
            share: differing / (p.length / 4),
            diff: canvas.toDataURL("image/png").split(",")[1],
          };
        },
        [png.toString("base64"), want.toString("base64"), CHANNEL],
      );
      if (result.size) failed.push(`${name}: size ${result.size}`);
      else if (result.share > SHARE) {
        failed.push(`${name}: ${(result.share * 100).toFixed(1)}% differ`);
        await mkdir(OUT, { recursive: true });
        await writeFile(join(OUT, `${name}.png`), png);
        await writeFile(
          join(OUT, `${name}.diff.png`),
          Buffer.from(result.diff, "base64"),
        );
      }
    }
    const stale = (await readdir(BASELINE)).filter(
      (file) => !shots.some(([name]) => `${name}.png` === file),
    );
    for (const file of stale) failed.push(`${file}: no longer drawn`);
    if (failed.length) {
      console.error(`field gallery: ${failed.length} mismatch(es)`);
      for (const line of failed) console.error(`  ${line}`);
      process.exitCode = 1;
    } else console.log(`field gallery: ${shots.length} pictures match`);
  }
} finally {
  await browser?.close();
  await rm(out, { recursive: true, force: true });
}
