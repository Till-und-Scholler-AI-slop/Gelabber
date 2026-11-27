// Reference screenshots of the web client for the native parity check
// (desktop/parity): every scene in scenes.json, signed in as its user, at
// the scene size, device scale 1, reduced motion, no caret. Scenes with
// `webOnly` (the "Betrieb" pages, which the native app leaves to the
// browser) are shot here too, as the web reference; desktop/parity skips them.
// Run in web/ against a seeded stack (seed.mjs):
//   node scripts/parity/shots.mjs [out-dir] [scene,…]
// GELABBER_PARITY_BROWSER names the Chromium binary (default /usr/bin/chromium).
// GELABBER_PARITY_LOG_INVOKE=1 prints the app commands `update` scenes call.
/* global process, console, document, window, URL, setTimeout, structuredClone */
import { execFileSync, spawn } from "node:child_process";
import { existsSync } from "node:fs";
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
// "{invite}": the seeded invite's code, asked for when a scene needs it.
const fill = (path) =>
  path.replace(/\{(\w+)\}/g, (_, key) =>
    key === "invite"
      ? execFileSync(process.execPath, [join(here, "say.mjs"), "invite-code", "lena", "home"], {
          encoding: "utf8",
        }).trim()
      : ids[key],
  );
// "/…/" names a regular expression, for labels that carry measurements.
const named = (name) => (/^\/.+\/$/.test(name) ? new RegExp(name.slice(1, -1)) : name);

// A step that fails ends the run; the call it started must not stay behind.
let running = null;
process.on("exit", () => running?.kill("SIGTERM"));

/** call.mjs with `channel people`, resolved once everyone is in. */
function startCall(spec) {
  const child = spawn(process.execPath, [join(here, "call.mjs"), ...spec.split(" ")], {
    stdio: ["ignore", "pipe", "inherit"],
  });
  running = child;
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
  running = null;
  return new Promise((resolve) => {
    child.removeAllListeners("exit");
    child.on("exit", () => setTimeout(resolve, 1500));
    child.kill("SIGTERM");
  });
}

// Scenes with `desktop`: the web client as the Tauri app showed it, which the
// native app follows: without install offers (NO_INSTALL) and with the app's
// own "Server wechseln …" in the user menu (USER_MENU).
const NO_INSTALL = await readFile(join(repo, "desktop/parity/reference/no-install.css"), "utf8");
const USER_MENU = await readFile(join(repo, "desktop/parity/reference/user-menu.js"), "utf8");
// Scenes with `update` (and `desktop`): the update the app offers, as
// update.rs `update_state` reports it, and its notice, the BANNER script
// the Tauri app put into every page. The install it starts stays at work.
const BANNER = await readFile(join(repo, "desktop/parity/reference/update-banner.js"), "utf8");
const desktopToml = await readFile(join(repo, "desktop/Cargo.toml"), "utf8");
const updateState = (update) => ({
  current: desktopToml.match(/^version = "(.+)"$/m)[1],
  supported: true,
  reason: null,
  autoCheck: true,
  phase: "idle",
  available: null,
  notes: null,
  offer: true,
  busy: false,
  error: null,
  lastCheck: null,
  downloaded: 0,
  total: null,
  ...update,
});

await mkdir(OUT, { recursive: true });
const launch = (extra) =>
  chromium.launch({
    executablePath: process.env.GELABBER_PARITY_BROWSER ?? "/usr/bin/chromium",
    args: ["--font-render-hinting=none", "--disable-lcd-text", ...extra],
  });
const browser = await launch([]);
// Scenes with `media` join calls: fake devices there only, so the device
// lists of the other scenes stay as a machine without devices shows them.
// Scenes with `microphone` show the audio settings as after "Gerätenamen
// anzeigen": fake devices and the microphone allowed. The app names its
// devices without asking, so its pages have no such hint.
let mediaBrowser = null;

const sayWords = (words) =>
  execFileSync(process.execPath, [join(here, "say.mjs"), ...words.split(" ")], {
    stdio: "inherit",
  });

for (const scene of scenes) {
  if (ONLY && !ONLY.includes(scene.name)) continue;
  // Every scene starts from the reference state (reset.mjs).
  execFileSync(process.execPath, [join(here, "reset.mjs")], { stdio: "inherit" });
  let restoreLocale = null;
  // say.mjs words that set the scene up (a theme), undone by the next reset.
  if (scene.before) sayWords(scene.before);
  const devices = scene.media || scene.microphone;
  if (devices && !mediaBrowser)
    mediaBrowser = await launch(["--use-fake-device-for-media-stream", "--use-fake-ui-for-media-stream"]);
  const context = await (devices ? mediaBrowser : browser).newContext({
    permissions: scene.media ? ["microphone", "camera"] : scene.microphone ? ["microphone"] : [],
    baseURL: fixture.base,
    viewport: { width: size[0], height: size[1] },
    deviceScaleFactor: 1,
    reducedMotion: "reduce",
    // `locale: "en"`: an English browser, as a signed-out scene needs it.
    locale: scene.locale === "en" ? "en-GB" : "de-DE",
    timezoneId: "Europe/Berlin",
    // Playwright's default; the native tool sets the same system theme.
    colorScheme: scene.scheme ?? "light",
    // `userAgent`: what the scene's sign-in records as its device, such as
    // the native app's, so Sicherheit names this device as the native one.
    userAgent: scene.userAgent,
  });
  if (scene.user) {
    const user = fixture.users[scene.user];
    const session = await (await context.request.get("/api/auth/session")).json();
    const login = await context.request.post("/api/auth/login", {
      data: { email: user.email, password: fixture.password },
      headers: { "X-CSRF-Token": session.csrf_token },
    });
    if (!login.ok()) throw new Error(`login ${scene.user}: ${login.status()}`);
    // `locale` on a signed-in scene: the account's language, German again
    // after the shot (and reset.mjs puts it back should a scene fail early).
    if (scene.locale) {
      const { csrf_token } = await login.json();
      const setLocale = (locale) =>
        context.request.put("/api/me/locale", {
          data: { locale },
          headers: { "X-CSRF-Token": csrf_token },
        });
      const set = await setLocale(scene.locale);
      if (!set.ok()) throw new Error(`locale ${scene.user}: ${set.status()}`);
      restoreLocale = () => setLocale("de");
    }
  }
  const call = scene.call ? await startCall(scene.call) : null;
  // A missing control fails the scene instead of waiting for ever.
  context.setDefaultTimeout(20_000);
  const page = await context.newPage();
  if (scene.page) {
    // A bundled page of the desktop app (its setup page), outside Tauri: its
    // commands fail as on Linux, unless the scene answers `update_state` as
    // the Windows setup does.
    await page.addInitScript((updates) => {
      window.__TAURI_INTERNALS__ = {
        invoke: (command) =>
          updates && command === "update_state"
            ? Promise.resolve({ supported: true, phase: "idle", error: null, ...updates })
            : Promise.reject(new Error("no app")),
      };
    }, scene.updates ?? null);
    // Served from an origin of its own: Chromium runs no module scripts
    // from file:// pages.
    const origin = "http://desktop-page.localhost";
    const dir = dirname(join(repo, scene.page));
    await page.route(`${origin}/**`, async (route) => {
      const path = new URL(route.request().url()).pathname;
      // The page's own files, else the web client's artwork, icons and fonts.
      const roots = [dir, join(repo, "web/public"), join(repo, "web/src")];
      const file = roots.map((root) => join(root, path)).find((f) => existsSync(f));
      if (!file) return route.fulfill({ status: 404 });
      const type = {
        ".html": "text/html",
        ".css": "text/css",
        ".js": "text/javascript",
        ".png": "image/png",
        ".woff2": "font/woff2",
      };
      await route.fulfill({
        body: await readFile(file),
        contentType: type[extname(file)] ?? "application/octet-stream",
      });
    });
    await page.goto(`${origin}/${basename(scene.page)}${scene.query ?? ""}`, {
      waitUntil: "networkidle",
    });
  } else {
    // The BANNER asks the app through Tauri's internals; each command is
    // answered as update.rs would.
    if (scene.update)
      await page.addInitScript(
        ({ answers, log }) => {
          window.__TAURI_INTERNALS__ = {
            invoke(command, args) {
              if (log) console.log(`invoke ${command} ${JSON.stringify(args)}`);
              if (command === "update_install") return new Promise(() => {});
              if (Object.hasOwn(answers, command))
                return Promise.resolve(structuredClone(answers[command]));
              return Promise.reject(`Command ${command} not found`);
            },
          };
        },
        {
          answers: { update_state: updateState(scene.update), update_dismiss: null },
          log: Boolean(process.env.GELABBER_PARITY_LOG_INVOKE),
        },
      );
    // After the shim: the BANNER looks for it when it runs. Its shadow root
    // opened, as Playwright finds buttons in open ones only.
    if (scene.update)
      await page.addInitScript({
        content: `{
          const attach = Element.prototype.attachShadow;
          Element.prototype.attachShadow = function (init) {
            return attach.call(this, { ...init, mode: "open" });
          };
        }
        ${BANNER}`,
      });
    if (scene.desktop) await page.addInitScript({ content: USER_MENU });
    // The desktop-app toast comes after a delay, so only the longer scenes
    // caught it, and the app never shows it: mark it as shown (desktopApp.ts).
    await page.addInitScript(() => {
      try {
        window.localStorage.setItem("gelabber.desktopApp.offered", "1");
      } catch {
        // about:blank has no storage; the scene's own page sets it.
      }
    });
    if (process.env.GELABBER_PARITY_LOG_INVOKE)
      page.on("console", (message) => {
        if (message.text().startsWith("invoke ")) console.log(`${scene.name}: ${message.text()}`);
      });
    await page.goto(fill(scene.path), { waitUntil: "networkidle" });
    if (scene.desktop) await page.addStyleTag({ content: NO_INSTALL });
  }
  await page.evaluate(() => document.fonts.ready);
  // A theme set by `before` arrives with the account's theme sync, which can
  // land after the network went quiet: wait until the page shows it.
  const [verb, , ...spec] = scene.before?.split(" ") ?? [];
  if (verb === "theme") {
    const text = spec.join(" ");
    const id = text.startsWith("{") ? JSON.parse(text).id : text;
    if (id !== "system")
      await page.waitForFunction((want) => document.documentElement.dataset.theme === want, id);
  }
  // Messages the scene posts (say.mjs), deleted after the screenshot, also
  // when a step fails, so the fixture stays as seeded.
  const said = [];
  // A hover keeps the pointer where it put it for the screenshot.
  let hovered = false;
  // Calls an action started while the page was open (`call`).
  const calls = [];
  try {
    for (const action of scene.actions ?? []) {
      // "channel people" for call.mjs, as a scene's `call`, but now.
      if (action.call) calls.push(await startCall(action.call));
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
      // A button, or a <summary> with that label (the user menu), which
      // Playwright does not expose as a button.
      if (action.click)
        await page
          .getByRole("button", { name: named(action.click), exact: true })
          .or(page.locator("summary").and(page.getByLabel(named(action.click), { exact: true })))
          .first()
          .click();
      if (action.menuitem)
        await page.getByRole("menuitem", { name: named(action.menuitem), exact: true }).first().click();
      if (action.link)
        await page.getByRole("link", { name: named(action.link), exact: true }).first().click();
      // The first button or link of that name, with the secondary button.
      if (action.rightclick)
        await page
          .getByRole("button", { name: named(action.rightclick), exact: true })
          .or(page.getByRole("link", { name: named(action.rightclick), exact: true }))
          .first()
          .click({ button: "right" });
      // Elements without a button role, such as a <summary>.
      if (action.clicktext)
        await page.getByText(named(action.clicktext), { exact: true }).first().click();
      // The first button or link, else the first element, with this name.
      if (action.hover) {
        await page
          .getByRole("button", { name: named(action.hover), exact: true })
          .or(page.getByRole("link", { name: named(action.hover), exact: true }))
          .or(page.getByText(named(action.hover), { exact: true }))
          .first()
          .hover();
        hovered = true;
      }
      // Selects the whole text of the first element with exactly this text.
      if (action.select)
        await page.getByText(action.select, { exact: true }).first().selectText();
      // A text field by its placeholder, such as the header's "Suchen".
      if (action.placeholder)
        await page.getByPlaceholder(action.placeholder, { exact: true }).first().click();
      // The text field with this accessible name (a label or its placeholder).
      if (action.focus)
        await page
          .getByRole("textbox", { name: action.focus, exact: true })
          .or(page.getByLabel(action.focus, { exact: true }))
          .or(page.getByPlaceholder(action.focus, { exact: true }))
          .first()
          .focus();
      // Playwright key names, e.g. "Control+k", "ArrowDown", "Escape".
      if (action.key) await page.keyboard.press(action.key);
      // Typed into whatever has the focus.
      if (action.type) await page.keyboard.type(action.type);
      if (action.wait) await page.waitForTimeout(action.wait);
      // The browser's back (Alt+← or the mouse's back button).
      if (action.back) await page.goBack();
      await page.waitForLoadState("networkidle");
    }
    await page.addStyleTag({ content: "*{caret-color:transparent !important}" });
    // Let lists settle and late images decode.
    await page.waitForTimeout(800);
    if (!hovered) {
      await page.mouse.move(size[0] - 1, size[1] - 1);
      // Long enough for hover fades and a tooltip's grace time to end, as
      // the native render waits too.
      await page.waitForTimeout(300);
    }
    await page.screenshot({ path: join(OUT, `${scene.name}.png`) });
  } finally {
    for (const [who, id] of said)
      execFileSync(process.execPath, [join(here, "say.mjs"), "delete", who, id], {
        stdio: "inherit",
      });
    // say.mjs words that undo what the scene's dialogs made (invite links).
    if (scene.after) sayWords(scene.after);
    // And what `before` set, so the last scene leaves no theme behind.
    if (scene.before)
      execFileSync(process.execPath, [join(here, "reset.mjs")], { stdio: "inherit" });
    for (const started of calls) await stopCall(started);
    if (restoreLocale) await restoreLocale();
  }
  console.log(`${scene.name}: ${page.url()}`);
  await context.close();
  if (call) await stopCall(call);
}
await browser.close();
await mediaBrowser?.close();
