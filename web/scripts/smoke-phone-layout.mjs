// Phone layout in a real engine, without a server, an account or the
// network: headless Chromium with a touch screen runs the built app from a
// made-up origin; its files, /api and the gateway are answered from here.
// It checks what the unit tests cannot see:
//  - the composer sized by script (field-sizing switched off, the way of iOS
//    before 26.2): the message list stays at its end while a draft wraps and
//    is sent, and the field is as tall as the stylesheet makes it, also
//    after the phone was turned;
//  - a phone on its side in a call whose sound the browser blocks: "Ton
//    starten" is on screen, in the sidebar's call card and in the dock.
// WebKit itself is not covered here; real phones remain device checks.
// Run in web/: node scripts/smoke-phone-layout.mjs
// GELABBER_PHONE_BROWSER_EXECUTABLE names a Chromium other than Playwright's.
/* global process, console, window, document, CSS, MediaDevices, URL, innerHeight, getComputedStyle, requestAnimationFrame */
import assert from "node:assert/strict";
import { mkdtemp, readFile, readdir, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { chromium, devices } from "playwright";
import { build } from "vite";

const web = fileURLToPath(new URL("..", import.meta.url));
// Secure, as the session's Web Locks need it, and without a server behind it.
const ORIGIN = "https://phone.test";
/** Built next to the app and loaded into its page: `window.smokeCall`. */
const PAGE = "scripts/smoke-phone-layout.page.mjs";

const id = (n) => `00000000-0000-4000-8000-${String(n).padStart(12, "0")}`;
const ME = id(6);
const JONAS = id(7);
const SERVER = id(101);
const TEXT = id(201);
const VOICE = id(202);
const AT = "2026-01-01T00:00:00Z";
const permissions = ["send_messages", "send_files", "join_voice"];
const server = {
  id: SERVER,
  name: "Wohnzimmer",
  owner_id: JONAS,
  created_at: AT,
  role: "member",
  permissions,
  member_permissions: permissions,
};
const channel = (channelId, name, kind) => ({
  id: channelId,
  server_id: SERVER,
  category_id: null,
  name,
  kind,
  created_at: AT,
});
const person = (userId, name) => ({ id: userId, name, avatar_url: null });
const detail = {
  ...server,
  categories: [],
  channels: [
    channel(TEXT, "allgemein", "text"),
    channel(VOICE, "Sofa", "voice"),
    // More than the sidebar of a phone on its side has room for.
    ...Array.from({ length: 8 }, (_, i) =>
      channel(id(210 + i), `spiel-${i}`, "text"),
    ),
  ],
  members: [
    {
      user_id: ME,
      name: "Lena",
      avatar_url: null,
      joined_at: AT,
      role: "member",
    },
    {
      user_id: JONAS,
      name: "Jonas",
      avatar_url: null,
      joined_at: AT,
      role: "owner",
    },
  ],
};
// More history than fits the screen, so the list has an end to stay at.
const history = Array.from({ length: 24 }, (_, i) => ({
  id: id(1000 + i),
  revision: 1,
  created_order: i + 1,
  channel_id: TEXT,
  author: i % 2 ? person(ME, "Lena") : person(JONAS, "Jonas"),
  content: `Nachricht ${i}: etwas Text, der auf dem Handy in die nächste Zeile umbricht.`,
  created_at: new Date(Date.UTC(2026, 0, 1, 17, i)).toISOString(),
  edited_at: null,
  attachments: [],
  reactions: [],
}));

function answer(path, method, body) {
  if (path === "/api/auth/session")
    return {
      user: { ...person(ME, "Lena"), email: "lena@example.test" },
      csrf_token: "a".repeat(64),
    };
  if (path === "/api/servers") return [server];
  if (path === `/api/servers/${SERVER}`) return detail;
  if (path === "/api/dms" || path === "/api/messages/unread") return [];
  if (path === "/api/me/themes")
    return { version: 1, revision: 1, active: "dark", customThemes: [] };
  if (path === `/api/channels/${TEXT}/messages`)
    return method === "GET"
      ? { messages: history, has_more: false }
      : {
          ...history[0],
          id: id(9000),
          created_order: history.length + 1,
          author: person(ME, "Lena"),
          content: JSON.parse(body ?? "{}").content ?? "",
          created_at: new Date(Date.UTC(2026, 0, 1, 18)).toISOString(),
        };
  if (path.endsWith("/read"))
    return {
      channel_id: TEXT,
      server_id: SERVER,
      read_message_id: history.at(-1).id,
      read_at: AT,
      unread_count: 0,
    };
  return {};
}

const out = await mkdtemp(join(tmpdir(), "gelabber-phone-layout-"));
/** What the build wrote, as the paths a browser asks for. */
let files;
/** Where the build put PAGE. */
let callState;
let browser;

const UPRIGHT = { width: 390, height: 664 };
const SIDEWAYS = { width: 844, height: 390 };
const problems = [];

/** A signed-in phone on the text channel. `init` runs before the app. */
async function phone(viewport, init) {
  const context = await browser.newContext({
    ...devices["iPhone 13"],
    viewport,
    locale: "de-DE",
    serviceWorkers: "block",
  });
  await context.route("**/*", (route) => {
    const request = route.request();
    const url = new URL(request.url());
    if (url.origin !== ORIGIN) return route.abort();
    if (url.pathname.startsWith("/api/"))
      return route.fulfill({
        contentType: "application/json",
        body: JSON.stringify(
          answer(url.pathname, request.method(), request.postData()),
        ),
      });
    // Any other path is a page of the app and gets its shell.
    return route.fulfill({
      path: join(out, files.has(url.pathname) ? url.pathname : "index.html"),
    });
  });
  await context.routeWebSocket("**/ws", (socket) => {
    socket.onMessage((raw) => {
      const frame = JSON.parse(raw);
      if (frame.op === "s")
        socket.send(JSON.stringify({ op: "ok", s: frame.s, c: frame.c, n: 0 }));
    });
  });
  await context.addInitScript(() => {
    // A phone's browser shares no screen; the call controls follow that.
    delete MediaDevices.prototype.getDisplayMedia;
    // A ResizeObserver loop is reported to the window, not thrown.
    window.smokeErrors = [];
    window.addEventListener("error", (event) =>
      window.smokeErrors.push(event.message),
    );
  });
  if (init) await context.addInitScript(init);
  const page = await context.newPage();
  page.setDefaultTimeout(15_000);
  page.on("pageerror", (error) => problems.push(error.message));
  await page.goto(`${ORIGIN}/s/${SERVER}/c/${TEXT}`);
  await page.locator(".lr-composer textarea").waitFor();
  await page.getByText(history.at(-1).content).waitFor();
  await frames(page);
  assert.ok(
    await page.evaluate(() => window.matchMedia("(pointer: coarse)").matches),
    "the emulated phone has no touch pointer",
  );
  return { context, page };
}

/** Lets layout, observers and the frames they ask for run. */
const frames = (page, count = 4) =>
  page.evaluate(async (left) => {
    while (left-- > 0) await new Promise(requestAnimationFrame);
  }, count);

async function done({ context, page }) {
  problems.push(...(await page.evaluate(() => window.smokeErrors)));
  await context.close();
}

/** The list against its end, the composer's field, the newest message. */
const chat = (page) =>
  page.evaluate(() => {
    const log = document.querySelector('[role="log"]');
    const field = document.querySelector(".lr-composer textarea");
    const last = [...document.querySelectorAll(".lr-message-row")].at(-1);
    return {
      fromEnd: Math.round(log.scrollHeight - log.scrollTop - log.clientHeight),
      pill: document.querySelector(".lr-jump-latest") !== null,
      field: field.offsetHeight,
      byScript: field.style.height !== "",
      sizing: getComputedStyle(field).fieldSizing,
      newest: last?.querySelector(".lr-message-text")?.textContent ?? "",
      newestBelowList: last
        ? Math.round(
            last.getBoundingClientRect().bottom -
              log.getBoundingClientRect().bottom,
          )
        : null,
    };
  });

async function atEnd(page, when) {
  await frames(page);
  const state = await chat(page);
  assert.ok(
    state.fromEnd <= 1 && !state.pill,
    `${when}: the list is ${state.fromEnd}px from its end${state.pill ? ' and offers "Zu den neuesten Nachrichten"' : ""}`,
  );
  return state;
}

const DRAFT =
  "Das ist ein Satz, der auf dem Handy in die zweite Zeile umbricht, dann weitergeht bis zur dritten und danach noch ein Stück in die vierte Zeile hinein.";

/** Types DRAFT word by word, turns the phone and back, sends by tap.
 * Returns the field's height after each step. */
async function draft(byScript) {
  const who = byScript ? "script" : "stylesheet";
  const device = await phone(
    UPRIGHT,
    byScript &&
      (() => {
        // A browser without field-sizing: unknown to script and stylesheet.
        const supports = CSS.supports.bind(CSS);
        CSS.supports = (...query) =>
          !String(query[0]).includes("field-sizing") && supports(...query);
        document.addEventListener("DOMContentLoaded", () => {
          const style = document.createElement("style");
          style.textContent = "textarea { field-sizing: fixed !important; }";
          document.head.append(style);
        });
      }),
  );
  const { page } = device;
  const start = await atEnd(page, `${who}, channel opened`);
  assert.equal(start.byScript, byScript, `${who}: who sizes the field`);
  assert.equal(start.sizing, byScript ? "fixed" : "content");
  const heights = [start.field];

  await page.tap(".lr-composer textarea");
  for (const [index, word] of DRAFT.split(" ").entries()) {
    await page.keyboard.type(index ? ` ${word}` : word, { delay: 5 });
    heights.push((await atEnd(page, `${who}, after "${word}"`)).field);
  }
  assert.ok(
    Math.max(...heights) >= 34 + 3 * 22,
    `${who}: the draft was meant to wrap into four lines or more, heights ${heights}`,
  );

  // On its side the same draft takes fewer lines, upright as many as
  // before; 320px wide it is more than the field may show.
  for (const viewport of [SIDEWAYS, UPRIGHT, { ...UPRIGHT, width: 320 }]) {
    await page.setViewportSize(viewport);
    heights.push((await atEnd(page, `${who}, ${viewport.width}px wide`)).field);
  }
  await page.setViewportSize(UPRIGHT);
  await atEnd(page, `${who}, upright again`);

  await page.tap(".lr-composer-send");
  await page.locator(".lr-message-row").last().getByText(DRAFT).waitFor();
  const sent = await atEnd(page, `${who}, after sending`);
  assert.equal(sent.newest, DRAFT);
  assert.ok(
    sent.newestBelowList <= 1,
    `${who}: the sent message ends ${sent.newestBelowList}px below the list`,
  );
  heights.push(sent.field);
  await done(device);
  return heights;
}

const VOICE_STATES = {
  "in a call with a stream": {
    status: "joined",
    serverId: SERVER,
    channelId: VOICE,
    channelName: "Sofa",
    sourceSubscriptions: { [JONAS]: { l: true } },
  },
  "in a call": {
    status: "joined",
    serverId: SERVER,
    channelId: VOICE,
    channelName: "Sofa",
  },
  "watching a stream": {
    watching: true,
    watchServerId: SERVER,
    watchChannelId: VOICE,
    watchChannelName: "Sofa",
  },
};

/** Where the unblock row and the rows it shares its container with are. */
const call = (page, container) =>
  page.evaluate((selector) => {
    const box = (element) => {
      const rect = element?.getBoundingClientRect();
      return rect && rect.height > 0 ? rect : null;
    };
    const root = document.querySelector(selector);
    const button = root?.querySelector(".voice-session-playback button");
    const rect = box(button);
    return {
      button: rect && [Math.round(rect.top), Math.round(rect.bottom)],
      // Without scrolling: all of it on screen, and a tap near any of its
      // (rounded) corners is a tap on it.
      reachable:
        rect !== null &&
        rect.top >= 0 &&
        rect.bottom <= innerHeight &&
        [rect.left + 8, rect.right - 8].every((x) =>
          [rect.top + 8, rect.bottom - 8].every((y) =>
            button.contains(document.elementFromPoint(x, y)),
          ),
        ),
      others: [...(root?.children ?? [])]
        .filter((row) => !row.matches(".voice-session-playback"))
        .map((row) => box(row)?.top)
        .filter((top) => top !== undefined),
      height: innerHeight,
    };
  }, container);

async function blockedSound() {
  const sizes = [
    // Wide enough for the sidebar and its call card.
    [844, 390, ".voice-session-card"],
    [915, 412, ".voice-session-card"],
    // Narrower phones keep the dock.
    [740, 360, ".voice-session-dock"],
    [667, 375, ".voice-session-dock"],
    // Upright: the row keeps its place below the controls.
    [390, 664, ".voice-session-dock"],
  ];
  for (const [width, height, container] of sizes) {
    const device = await phone({ width, height });
    const { page } = device;
    await page.addScriptTag({ type: "module", url: callState });
    await page.waitForFunction(() => window.smokeCall);
    for (const [name, state] of Object.entries(VOICE_STATES)) {
      await page.evaluate((next) => window.smokeCall(next), {
        ...state,
        playbackBlocked: true,
      });
      await page.locator(`${container} .voice-session-playback`).waitFor();
      await frames(page);
      const seen = await call(page, container);
      const where = `${width}x${height}, ${name}`;
      assert.ok(
        seen.reachable,
        `${where}: "Ton starten" at ${seen.button} is not on a screen ${seen.height}px high`,
      );
      if (height > width)
        assert.ok(
          seen.others.every((top) => top < seen.button[0]),
          `${where}: upright, the unblock row belongs below the controls`,
        );
    }
    await done(device);
  }
}

try {
  // The app as it ships, and PAGE in the same build so both share modules.
  await build({
    root: web,
    logLevel: "error",
    build: {
      outDir: out,
      emptyOutDir: true,
      manifest: true,
      rollupOptions: { input: [join(web, "index.html"), join(web, PAGE)] },
    },
  });
  files = new Set(
    (await readdir(out, { recursive: true })).map((name) => `/${name}`),
  );
  const manifest = JSON.parse(
    await readFile(join(out, ".vite/manifest.json"), "utf8"),
  );
  callState = `/${manifest[PAGE].file}`;
  browser = await chromium.launch({
    executablePath: process.env.GELABBER_PHONE_BROWSER_EXECUTABLE || undefined,
    // Never a window, and nothing on the desktop's session bus.
    headless: true,
    env: { ...process.env, DBUS_SESSION_BUS_ADDRESS: "disabled:" },
  });

  const stylesheet = await draft(false);
  console.log(
    `PASS: sized by the stylesheet, the list stays at its end (field up to ${Math.max(...stylesheet)}px)`,
  );
  const script = await draft(true);
  console.log(
    "PASS: sized by script, the list stays at its end while a draft wraps, the phone turns and the message is sent",
  );
  assert.deepEqual(
    script,
    stylesheet,
    "the script sizes the field differently from the stylesheet",
  );
  console.log("PASS: the script's heights are the stylesheet's at every step");
  await blockedSound();
  console.log(
    'PASS: "Ton starten" is on screen on a phone on its side, in the call card and in the dock',
  );
  assert.deepEqual(problems, [], "the page reported errors");
  console.log(
    `PASS: no page errors, ${browser.browserType().name()} ${browser.version()}`,
  );
} finally {
  await browser?.close();
  await rm(out, { recursive: true, force: true });
}
