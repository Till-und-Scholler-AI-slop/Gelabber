// Reference screenshots of the web client for the native parity check
// (desktop/parity): every scene in scenes.json, signed in as its user, at
// the scene size, device scale 1, reduced motion, no caret.
// Run in web/ against a seeded stack (seed.mjs):
//   node scripts/parity/shots.mjs [out-dir] [scene,…]
// GELABBER_PARITY_BROWSER names the Chromium binary (default /usr/bin/chromium).
/* global process, console, document, window, URL, setTimeout */
import { execFileSync, spawn } from "node:child_process";
import { mkdir, readFile } from "node:fs/promises";
import { basename, dirname, extname, join } from "node:path";
import { chromium } from "playwright";

const here = new URL(".", import.meta.url).pathname;
const repo = join(here, "../../..");
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

/** call.mjs with `channel people`, resolved once everyone is in. */
function startCall(spec) {
  const child = spawn(process.execPath, [join(here, "call.mjs"), ...spec.split(" ")], {
    stdio: ["ignore", "pipe", "inherit"],
  });
  return new Promise((resolve, reject) => {
    let out = "";
    child.stdout.on("data", (chunk) => {
      out += chunk;
      if (out.includes("in the call")) setTimeout(() => resolve(child), 1500);
    });
    child.on("exit", (code) => reject(new Error(`call.mjs exited with ${code}`)));
  });
}

function stopCall(child) {
  return new Promise((resolve) => {
    child.removeAllListeners("exit");
    child.on("exit", () => setTimeout(resolve, 1500));
    child.kill("SIGTERM");
  });
}

await mkdir(OUT, { recursive: true });
const launch = (extra) =>
  chromium.launch({
    executablePath: process.env.GELABBER_PARITY_BROWSER ?? "/usr/bin/chromium",
    args: ["--font-render-hinting=none", "--disable-lcd-text", ...extra],
  });
const browser = await launch([]);
// Scenes with `media` join calls: fake devices there only, so the device
// lists of the other scenes stay as a machine without devices shows them.
let mediaBrowser = null;

const sayWords = (words) =>
  execFileSync(process.execPath, [join(here, "say.mjs"), ...words.split(" ")], {
    stdio: "inherit",
  });

for (const scene of scenes) {
  if (ONLY && !ONLY.includes(scene.name)) continue;
  // Every scene starts from the reference state (reset.mjs).
  execFileSync(process.execPath, [join(here, "reset.mjs")], { stdio: "inherit" });
  // say.mjs words that set the scene up (a theme), undone by the next reset.
  if (scene.before) sayWords(scene.before);
  if (scene.media && !mediaBrowser)
    mediaBrowser = await launch(["--use-fake-device-for-media-stream", "--use-fake-ui-for-media-stream"]);
  const context = await (scene.media ? mediaBrowser : browser).newContext({
    permissions: scene.media ? ["microphone", "camera"] : [],
    baseURL: fixture.base,
    viewport: { width: size[0], height: size[1] },
    deviceScaleFactor: 1,
    reducedMotion: "reduce",
    locale: "de-DE",
    timezoneId: "Europe/Berlin",
    // Playwright's default; the native tool sets the same system theme.
    colorScheme: scene.scheme ?? "light",
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
  const call = scene.call ? await startCall(scene.call) : null;
  const page = await context.newPage();
  if (scene.page) {
    // A bundled page of the desktop app (its setup page), outside Tauri:
    // its commands fail as on Linux, where updates are the package manager's.
    await page.addInitScript(() => {
      window.__TAURI_INTERNALS__ = { invoke: () => Promise.reject(new Error("no app")) };
    });
    // Served from an origin of its own: Chromium runs no module scripts
    // from file:// pages.
    const origin = "http://desktop-page.localhost";
    const dir = dirname(join(repo, scene.page));
    await page.route(`${origin}/**`, async (route) => {
      const file = join(dir, new URL(route.request().url()).pathname);
      const type = { ".html": "text/html", ".css": "text/css", ".js": "text/javascript" };
      await route.fulfill({
        body: await readFile(file),
        contentType: type[extname(file)] ?? "application/octet-stream",
      });
    });
    await page.goto(`${origin}/${basename(scene.page)}${scene.query ?? ""}`, {
      waitUntil: "networkidle",
    });
  } else {
    await page.goto(fill(scene.path), { waitUntil: "networkidle" });
  }
  await page.evaluate(() => document.fonts.ready);
  // Messages the scene posts (say.mjs), deleted after the screenshot, also
  // when a step fails, so the fixture stays as seeded.
  const said = [];
  // A hover keeps the pointer where it put it for the screenshot.
  let hovered = false;
  try {
    for (const action of scene.actions ?? []) {
      // "<user> <channel-key> <text>": someone else writes.
      if (action.say) {
        const [who, where, ...words] = action.say.split(" ");
        const id = execFileSync(
          process.execPath,
          [join(here, "say.mjs"), "post", who, where, words.join(" ")],
          { encoding: "utf8" },
        ).trim();
        said.push([who, id]);
      }
      if (action.click)
        await page.getByRole("button", { name: action.click, exact: true }).first().click();
      if (action.link)
        await page.getByRole("link", { name: action.link, exact: true }).first().click();
      // The first button or link of that name, with the secondary button.
      if (action.rightclick)
        await page
          .getByRole("button", { name: action.rightclick, exact: true })
          .or(page.getByRole("link", { name: action.rightclick, exact: true }))
          .first()
          .click({ button: "right" });
      // Elements without a button role, such as a <summary>.
      if (action.clicktext)
        await page.getByText(action.clicktext, { exact: true }).first().click();
      // The first button or link, else the first element, with this name.
      if (action.hover) {
        await page
          .getByRole("button", { name: action.hover, exact: true })
          .or(page.getByRole("link", { name: action.hover, exact: true }))
          .or(page.getByText(action.hover, { exact: true }))
          .first()
          .hover();
        hovered = true;
      }
      // Playwright key names, e.g. "Control+k", "ArrowDown", "Escape".
      if (action.key) await page.keyboard.press(action.key);
      // Typed into whatever has the focus.
      if (action.type) await page.keyboard.type(action.type);
      if (action.wait) await page.waitForTimeout(action.wait);
      await page.waitForLoadState("networkidle");
    }
    await page.addStyleTag({ content: "*{caret-color:transparent !important}" });
    // Let lists settle and late images decode.
    await page.waitForTimeout(800);
    if (!hovered) await page.mouse.move(size[0] - 1, size[1] - 1);
    await page.screenshot({ path: join(OUT, `${scene.name}.png`) });
  } finally {
    for (const [who, id] of said)
      execFileSync(process.execPath, [join(here, "say.mjs"), "delete", who, id], {
        stdio: "inherit",
      });
    // say.mjs words that undo what the scene's dialogs made (invite links).
    if (scene.after) sayWords(scene.after);
  }
  console.log(`${scene.name}: ${page.url()}`);
  await context.close();
  if (call) await stopCall(call);
}
await browser.close();
await mediaBrowser?.close();
