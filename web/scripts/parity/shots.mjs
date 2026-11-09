// Reference screenshots of the web client for the native parity check
// (desktop/parity): every scene in scenes.json, signed in as its user, at
// the scene size, device scale 1, reduced motion, no caret.
// Run in web/ against a seeded stack (seed.mjs):
//   node scripts/parity/shots.mjs [out-dir] [scene,…]
// GELABBER_PARITY_BROWSER names the Chromium binary (default /usr/bin/chromium).
// GELABBER_PARITY_LOG_INVOKE=1 prints the app commands `desktop` scenes call.
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

// Scenes with `desktop`: the page inside the Tauri app, each command answered
// as that app's would. The web client now renders its browser branches there
// and asks only `media_info` (tauriApp.ts); the update banner asks `update_*`.
const desktopToml = await readFile(join(repo, "desktop/Cargo.toml"), "utf8");
const ffi = await readFile(join(repo, "desktop/core/src/ffi.rs"), "utf8");
const DESKTOP = {
  // media.rs media_info: FEATURES on Linux; this machine's codecs, no hardware.
  media_info: {
    abi: Number(ffi.match(/GM_ABI_VERSION: u32 = (\d+)/)[1]),
    version: desktopToml.match(/^version = "(.+)"$/m)[1],
    platform: "linux",
    features: [
      "screen",
      "camera",
      "app-audio",
      "app-audio-excludes-self",
      "video-frames",
      "vp9-svc",
    ],
    videoCodecs: Object.fromEntries(
      ["av1", "h264"].map((codec) => [
        codec,
        { decoder: null, encoder: null, hardwareDecoder: false, hardwareEncoder: false },
      ]),
    ),
  },
  // The core's dummy audio, as the native render runs it (GELABBER_FAKE_MEDIA).
  media_audio_devices: { input: "", inputs: [], output: "", outputs: [] },
  media_video_devices: [],
  // notify.rs: the system shows them, as in the native app.
  notify_info: { available: true },
  notify_close: null,
  // shell.rs: no tray here.
  shell_info: { tray: false, close_to_tray: false },
  shell_unread: null,
};
// Scenes with `update` (and `desktop`): the update the app offers, as
// update.rs `update_state` reports it, and its notice, the BANNER script
// the Tauri app put into every page. The install it starts stays at work.
const BANNER = await readFile(join(repo, "desktop/parity/reference/update-banner.js"), "utf8");
const updateState = (update) => ({
  current: DESKTOP.media_info.version,
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
// What the commands fail with here; every other command is not found.
const DESKTOP_ERRORS = {
  media_audio_apps: "native media core: application sound needs PipeWire (libpipewire-0.3)",
};

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
    if (scene.desktop)
      await page.addInitScript(
        ({ answers, errors, pending, log }) => {
          // @tauri-apps/api/core's contract: invoke, and ids for a Channel,
          // to which no answered command sends.
          let next = 0;
          window.__TAURI_INTERNALS__ = {
            transformCallback: () => ++next,
            unregisterCallback: () => {},
            invoke(command, args) {
              if (log) console.log(`invoke ${command} ${JSON.stringify(args)}`);
              if (pending.includes(command)) return new Promise(() => {});
              if (Object.hasOwn(answers, command))
                return Promise.resolve(structuredClone(answers[command]));
              return Promise.reject(errors[command] ?? `Command ${command} not found`);
            },
          };
        },
        {
          answers: scene.update
            ? { ...DESKTOP, update_state: updateState(scene.update), update_dismiss: null }
            : DESKTOP,
          errors: DESKTOP_ERRORS,
          pending: scene.update ? ["update_install"] : [],
          log: Boolean(process.env.GELABBER_PARITY_LOG_INVOKE),
        },
      );
    // After the shim: the BANNER looks for it when it runs. Its shadow root
    // opened, as Playwright finds buttons in open ones only.
    if (scene.desktop && scene.update)
      await page.addInitScript({
        content: `{
          const attach = Element.prototype.attachShadow;
          Element.prototype.attachShadow = function (init) {
            return attach.call(this, { ...init, mode: "open" });
          };
        }
        ${BANNER}`,
      });
    if (process.env.GELABBER_PARITY_LOG_INVOKE)
      page.on("console", (message) => {
        if (message.text().startsWith("invoke ")) console.log(`${scene.name}: ${message.text()}`);
      });
    await page.goto(fill(scene.path), { waitUntil: "networkidle" });
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
      if (action.click)
        await page.getByRole("button", { name: named(action.click), exact: true }).first().click();
      if (action.menuitem)
        await page.getByRole("menuitem", { name: named(action.menuitem), exact: true }).first().click();
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
        await page.getByText(named(action.clicktext), { exact: true }).first().click();
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
  }
  console.log(`${scene.name}: ${page.url()}`);
  await context.close();
  if (call) await stopCall(call);
}
await browser.close();
await mediaBrowser?.close();
