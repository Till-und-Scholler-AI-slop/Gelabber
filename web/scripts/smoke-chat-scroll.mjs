// Chat history paging in a real engine (#201), without a server, an account
// or the network: headless Chromium runs the built app from a made-up
// origin; its files, /api and the gateway are answered from here.
// It checks what the unit tests cannot see: the reader scrolls up until the
// older page is asked for and stops; that page arrives about 400 ms later,
// well after the last scroll event, with rows much taller than the list's
// estimate. The row at the top of the list must stay where it was, while
// the new rows are laid out and measured above it.
// Run in web/: node scripts/smoke-chat-scroll.mjs
// GELABBER_CHAT_BROWSER_EXECUTABLE names a Chromium other than Playwright's.
/* global process, console, window, document, URL, requestAnimationFrame, setTimeout */
import assert from "node:assert/strict";
import { mkdtemp, readdir, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { chromium } from "playwright";
import { build } from "vite";

const web = fileURLToPath(new URL("..", import.meta.url));
// Secure, as the session's Web Locks need it, and without a server behind it.
const ORIGIN = "https://chat.test";
/** How long the older page is held back after it was asked for. */
const HOLD_MS = 400;

const id = (n) => `00000000-0000-4000-8000-${String(n).padStart(12, "0")}`;
const ME = id(6);
const JONAS = id(7);
const SERVER = id(101);
const TEXT = id(201);
const AT = "2026-01-01T00:00:00Z";
const permissions = ["send_messages", "send_files"];
const server = {
  id: SERVER,
  name: "Wohnzimmer",
  owner_id: JONAS,
  created_at: AT,
  role: "member",
  permissions,
  member_permissions: permissions,
};
const person = (userId, name) => ({ id: userId, name, avatar_url: null });
const detail = {
  ...server,
  categories: [],
  channels: [
    {
      id: TEXT,
      server_id: SERVER,
      category_id: null,
      name: "allgemein",
      kind: "text",
      created_at: AT,
    },
  ],
  members: [
    { ...person(ME, "Lena"), user_id: ME, joined_at: AT, role: "member" },
    { ...person(JONAS, "Jonas"), user_id: JONAS, joined_at: AT, role: "owner" },
  ],
};
const WORDS =
  "Das ist ein ganz gewöhnlicher Satz in einer Unterhaltung, der eine Zeile füllt und dann weitergeht. ";
/** Message `n`, oldest first. Older ones are far taller than estimated. */
const message = (n) => ({
  id: id(1000 + n),
  revision: 1,
  created_order: n + 1,
  channel_id: TEXT,
  // Pairs by one author: some rows are continuations, estimated smaller.
  author: Math.floor(n / 2) % 2 ? person(ME, "Lena") : person(JONAS, "Jonas"),
  content: `Nachricht ${n}. ${WORDS.repeat(n < 50 ? 4 + (n % 5) * 3 : 1 + (n % 3))}`,
  created_at: new Date(Date.UTC(2026, 0, 1, 12, 0, n * 20)).toISOString(),
  edited_at: null,
  attachments: [],
  reactions: [],
});
const all = Array.from({ length: 100 }, (_, n) => message(n));
const newest = all.slice(50);
const older = all.slice(0, 50);

/** Settles once the older page was asked for; `release` answers it. */
let asked;
let olderAsked;
let releaseOlder;

function answer(path, method) {
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
  if (path.endsWith("/read") || method !== "GET")
    return {
      channel_id: TEXT,
      server_id: SERVER,
      read_message_id: newest.at(-1).id,
      read_at: AT,
      unread_count: 0,
    };
  return {};
}

const frames = (page, count = 4) =>
  page.evaluate(async (left) => {
    while (left-- > 0) await new Promise(requestAnimationFrame);
  }, count);

/** The first row whose bottom is inside the list, and its top in it. */
const firstVisible = (page) =>
  page.evaluate(() => {
    const log = document.querySelector('[role="log"]');
    const top = log.getBoundingClientRect().top;
    const rows = [...log.querySelectorAll("[data-index]")]
      .map((row) => ({ row, rect: row.getBoundingClientRect() }))
      .filter(({ rect }) => rect.bottom > top + 1)
      .sort((a, b) => a.rect.top - b.rect.top);
    const first = rows[0];
    const text =
      first?.row.querySelector(".lr-message-text")?.textContent ?? "";
    return {
      name: text.split(".")[0],
      top: first ? first.rect.top - top : null,
      scrollTop: log.scrollTop,
    };
  });

/** Where the row `name` is now, relative to the list's top. */
const rowTop = (page, name) =>
  page.evaluate((wanted) => {
    const log = document.querySelector('[role="log"]');
    const row = [...log.querySelectorAll("[data-index]")].find((candidate) =>
      candidate
        .querySelector(".lr-message-text")
        ?.textContent?.startsWith(`${wanted}.`),
    );
    return row
      ? row.getBoundingClientRect().top - log.getBoundingClientRect().top
      : null;
  }, name);

const out = await mkdtemp(join(tmpdir(), "gelabber-chat-scroll-"));
let browser;
const problems = [];

try {
  await build({
    root: web,
    logLevel: "error",
    build: { outDir: out, emptyOutDir: true },
  });
  const files = new Set(
    (await readdir(out, { recursive: true })).map((name) => `/${name}`),
  );
  browser = await chromium.launch({
    executablePath: process.env.GELABBER_CHAT_BROWSER_EXECUTABLE || undefined,
    headless: true,
    env: { ...process.env, DBUS_SESSION_BUS_ADDRESS: "disabled:" },
  });
  const context = await browser.newContext({
    viewport: { width: 1100, height: 760 },
    locale: "de-DE",
    serviceWorkers: "block",
  });
  asked = new Promise((resolve) => {
    olderAsked = resolve;
  });
  const held = new Promise((resolve) => {
    releaseOlder = resolve;
  });
  await context.route("**/*", async (route) => {
    const request = route.request();
    const url = new URL(request.url());
    if (url.origin !== ORIGIN) return route.abort();
    if (url.pathname === `/api/channels/${TEXT}/messages`) {
      if (!url.searchParams.has("before"))
        return route.fulfill({
          contentType: "application/json",
          body: JSON.stringify({ messages: newest, has_more: true }),
        });
      olderAsked(Date.now());
      await held;
      return route.fulfill({
        contentType: "application/json",
        body: JSON.stringify({ messages: older, has_more: false }),
      });
    }
    if (url.pathname.startsWith("/api/"))
      return route.fulfill({
        contentType: "application/json",
        body: JSON.stringify(answer(url.pathname, request.method())),
      });
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
    window.smokeErrors = [];
    window.addEventListener("error", (event) =>
      window.smokeErrors.push(event.message),
    );
  });
  const page = await context.newPage();
  page.setDefaultTimeout(15_000);
  page.on("pageerror", (error) => problems.push(error.message));
  await page.goto(`${ORIGIN}/s/${SERVER}/c/${TEXT}`);
  await page.getByText("Nachricht 99.").waitFor();
  await frames(page, 8);

  // Scroll up the way a reader does until the older page is asked for.
  const box = await page.locator('[role="log"]').boundingBox();
  await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2);
  let askedAt = null;
  asked.then((at) => {
    askedAt = at;
  });
  for (let step = 0; step < 200 && askedAt === null; step++) {
    await page.mouse.wheel(0, -240);
    await new Promise((resolve) => setTimeout(resolve, 40));
  }
  assert.ok(askedAt !== null, "scrolling up never asked for older history");

  // Stopped: let the last scroll settle, then note the top row.
  await frames(page, 6);
  const before = await firstVisible(page);
  assert.ok(before.top !== null, "no row at the top of the list");
  const wait = askedAt + HOLD_MS - Date.now();
  if (wait > 0) await new Promise((resolve) => setTimeout(resolve, wait));
  releaseOlder();

  // Prepended: each row's index is now its message number.
  await page.waitForFunction(() =>
    [...document.querySelectorAll('[role="log"] [data-index]')].some((row) => {
      const text = row.querySelector(".lr-message-text")?.textContent ?? "";
      return text.startsWith(`Nachricht ${row.dataset.index}.`);
    }),
  );
  // Layout, measurement of the new rows above, and their corrections.
  const seen = [];
  for (let pass = 0; pass < 12; pass++) {
    await frames(page, 4);
    seen.push(await rowTop(page, before.name));
  }
  const drift = seen.map((top) =>
    top === null ? "off screen" : Math.round(top - before.top),
  );
  assert.ok(
    drift.every((delta) => typeof delta === "number" && Math.abs(delta) <= 2),
    `"${before.name}" was ${Math.round(before.top)}px from the list's top and moved by ${drift.join(", ")} px while the older page was laid out`,
  );
  console.log(
    `PASS: "${before.name}" stays at ${Math.round(before.top)}px while the older page (held ${HOLD_MS} ms) is laid out and measured`,
  );

  problems.push(...(await page.evaluate(() => window.smokeErrors)));
  await context.close();
  // Scrolling up through rows not measured yet makes Chromium report a
  // ResizeObserver loop; main without #201 shows the same. Said, not failed.
  const loop = "ResizeObserver loop completed with undelivered notifications.";
  const loops = problems.filter((problem) => problem === loop).length;
  assert.deepEqual(
    problems.filter((problem) => problem !== loop),
    [],
    "the page reported errors",
  );
  console.log(
    `PASS: no page errors${loops ? ` (besides ${loops} ResizeObserver loop notices while scrolling)` : ""}, ${browser.browserType().name()} ${browser.version()}`,
  );
} finally {
  await browser?.close();
  await rm(out, { recursive: true, force: true });
}
