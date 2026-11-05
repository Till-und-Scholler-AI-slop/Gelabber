// Reference screenshots of the web client for the native parity check
// (desktop/parity): every scene in scenes.json, signed in as its user, at
// the scene size, device scale 1, reduced motion, no caret.
// Run in web/ against a seeded stack (seed.mjs):
//   node scripts/parity/shots.mjs [out-dir] [scene,…]
// GELABBER_PARITY_BROWSER names the Chromium binary (default /usr/bin/chromium).
/* global process, console, document */
import { mkdir, readFile } from "node:fs/promises";
import { join } from "node:path";
import { chromium } from "playwright";

const here = new URL(".", import.meta.url).pathname;
const OUT = process.argv[2] ?? "/tmp/gelabber-parity/web";
const ONLY = process.argv[3]?.split(",").filter(Boolean);
const fixture = JSON.parse(await readFile(join(here, "fixture.json"), "utf8"));
const { size, scenes } = JSON.parse(await readFile(join(here, "scenes.json"), "utf8"));

const ids = {
  home: fixture.servers.home.id,
  evening: fixture.servers.evening.id,
  dm: fixture.dm,
  ...fixture.servers.home.channels,
};
const fill = (path) => path.replace(/\{(\w+)\}/g, (_, key) => ids[key]);

await mkdir(OUT, { recursive: true });
const browser = await chromium.launch({
  executablePath: process.env.GELABBER_PARITY_BROWSER ?? "/usr/bin/chromium",
  args: ["--font-render-hinting=none", "--disable-lcd-text"],
});

for (const scene of scenes) {
  if (ONLY && !ONLY.includes(scene.name)) continue;
  const context = await browser.newContext({
    baseURL: fixture.base,
    viewport: { width: size[0], height: size[1] },
    deviceScaleFactor: 1,
    reducedMotion: "reduce",
    locale: "de-DE",
    timezoneId: "Europe/Berlin",
  });
  if (scene.user) {
    const user = fixture.users[scene.user];
    const session = await (await context.request.get("/api/auth/session")).json();
    const login = await context.request.post("/api/auth/login", {
      data: { email: user.email, password: fixture.password },
      headers: { "X-CSRF-Token": session.csrf_token },
    });
    if (!login.ok()) throw new Error(`login ${scene.user}: ${login.status()}`);
  }
  const page = await context.newPage();
  await page.goto(fill(scene.path), { waitUntil: "networkidle" });
  await page.evaluate(() => document.fonts.ready);
  await page.addStyleTag({ content: "*{caret-color:transparent !important}" });
  // Let lists settle and late images decode.
  await page.waitForTimeout(800);
  await page.mouse.move(size[0] - 1, size[1] - 1);
  await page.screenshot({ path: join(OUT, `${scene.name}.png`) });
  console.log(`${scene.name}: ${page.url()}`);
  await context.close();
}
await browser.close();
