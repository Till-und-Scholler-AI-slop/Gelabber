// Reference screenshots of the web client for the native parity check
// (desktop/parity): every scene in scenes.json, signed in as its user, at
// the scene size, device scale 1, reduced motion, no caret.
// Run in web/ against a seeded stack (seed.mjs):
//   node scripts/parity/shots.mjs [out-dir] [scene,…]
// GELABBER_PARITY_BROWSER names the Chromium binary (default /usr/bin/chromium).
/* global process, console, document */
import { execFileSync, spawn } from "node:child_process";
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

for (const scene of scenes) {
  if (ONLY && !ONLY.includes(scene.name)) continue;
  // Every scene starts from the reference read state (reset.mjs).
  execFileSync(process.execPath, [join(here, "reset.mjs")], { stdio: "inherit" });
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
  await page.goto(fill(scene.path), { waitUntil: "networkidle" });
  await page.evaluate(() => document.fonts.ready);
  // Messages the scene posts (say.mjs), deleted after the screenshot.
  const said = [];
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
  await page.mouse.move(size[0] - 1, size[1] - 1);
  await page.screenshot({ path: join(OUT, `${scene.name}.png`) });
  for (const [who, id] of said)
    execFileSync(process.execPath, [join(here, "say.mjs"), "delete", who, id], { stdio: "inherit" });
  console.log(`${scene.name}: ${page.url()}`);
  await context.close();
  if (call) await stopCall(call);
}
await browser.close();
await mediaBrowser?.close();
