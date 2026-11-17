// The member list with thousands of members online (#613), in a real
// engine and without a server: headless Chromium runs the built app from a
// made-up origin; its files, /api and the gateway are answered from here.
// It measures what the v0.11.0 load test saw grow (#610): the page's DOM
// nodes, its JS heap and, on Linux, the memory of the tab's renderer and of
// the whole browser. Then it checks that the virtual list stays usable: the
// arrow keys walk past the first screen, Enter opens a profile, the end of
// the list can be scrolled to, and focus survives the focused member going
// offline (their row leaves the page).
// Run in web/: node scripts/smoke-member-list.mjs
// GELABBER_MEMBER_LIST_COUNTS: members online per run, comma-separated
//   (default 2000). GELABBER_MEMBER_LIST_MEASURE_ONLY=1 skips the checks,
//   to measure a build without the virtual list.
// GELABBER_MEMBER_LIST_SHOTS: a folder for a picture of each run's list.
// GELABBER_MEMBER_LIST_BROWSER_EXECUTABLE names a Chromium other than
//   Playwright's.
/* global process, console, document, URL, requestAnimationFrame */
import assert from "node:assert/strict";
import { readFile, mkdtemp, readdir, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { chromium } from "playwright";
import { build } from "vite";

const web = fileURLToPath(new URL("..", import.meta.url));
// Secure, as the session's Web Locks need it, and without a server behind it.
const ORIGIN = "https://members.test";
const COUNTS = (process.env.GELABBER_MEMBER_LIST_COUNTS ?? "2000")
  .split(",")
  .map(Number);
const MEASURE_ONLY = process.env.GELABBER_MEMBER_LIST_MEASURE_ONLY === "1";

const id = (n) => `00000000-0000-4000-8000-${String(n).padStart(12, "0")}`;
const ME = id(1);
const SERVER = id(101);
const TEXT = id(201);
const AT = "2026-01-01T00:00:00Z";
const permissions = ["send_messages", "send_files"];
const server = {
  id: SERVER,
  name: "Lasttest",
  owner_id: ME,
  created_at: AT,
  role: "owner",
  permissions,
  member_permissions: permissions,
};
/** Member `n`, named so that the list's order is theirs. */
const member = (n) => ({
  user_id: id(10_000 + n),
  name: `Mitglied ${String(n).padStart(5, "0")}`,
  avatar_url: null,
  joined_at: AT,
  role: "member",
  role_ids: [],
  // Some rows are taller than the list's guess.
  status_text: n % 7 === 3 ? `Status von ${n}` : null,
  accent_color: null,
  avatar_decoration: null,
});

function detail(count) {
  return {
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
      {
        user_id: ME,
        name: "Beobachter",
        avatar_url: null,
        joined_at: AT,
        role: "owner",
        role_ids: [],
      },
      ...Array.from({ length: count }, (_, n) => member(n)),
    ],
  };
}

function answer(path, count) {
  if (path === "/api/auth/session")
    return {
      user: {
        id: ME,
        name: "Beobachter",
        avatar_url: null,
        email: "observer@example.test",
      },
      csrf_token: "a".repeat(64),
    };
  if (path === "/api/servers") return [server];
  if (path === `/api/servers/${SERVER}`) return detail(count);
  if (path === `/api/channels/${TEXT}/messages`)
    return { messages: [], has_more: false };
  if (
    path === "/api/messages/unread" ||
    path === "/api/dms" ||
    path === "/api/notification-settings" ||
    path === "/api/users/me/blocks"
  )
    return [];
  if (path === "/api/me/themes")
    return { version: 1, revision: 1, active: "dark", customThemes: [] };
  if (path === "/api/friends")
    return { friends: [], incoming: [], outgoing: [] };
  const profile = /^\/api\/users\/([^/]+)$/.exec(path)?.[1];
  if (profile) {
    const n = Number(profile.slice(-12)) - 10_000;
    const { user_id, name } = member(n);
    return { id: user_id, name, avatar_url: null, created_at: AT };
  }
  return {};
}

const frames = (page, count = 4) =>
  page.evaluate(async (left) => {
    while (left-- > 0) await new Promise(requestAnimationFrame);
  }, count);

/** Proportional set size of a process in MB, or null off Linux. */
async function pssMb(pid) {
  try {
    const text = await readFile(`/proc/${pid}/smaps_rollup`, "utf8");
    const kb = Number(/^Pss:\s+(\d+)/m.exec(text)?.[1]);
    return Number.isFinite(kb) ? kb / 1024 : null;
  } catch {
    return null;
  }
}

/**
 * Main-thread time one presence change costs: a member goes idle and comes
 * back, `rounds` times, and the page's task time is shared out.
 */
async function updateCost(page, presence, userId, rounds = 20) {
  const cdp = await page.context().newCDPSession(page);
  await cdp.send("Performance.enable");
  const taskTime = async () =>
    (await cdp.send("Performance.getMetrics")).metrics.find(
      (entry) => entry.name === "TaskDuration",
    ).value;
  await frames(page, 4);
  const before = await taskTime();
  for (let round = 0; round < rounds; round++) {
    presence(userId, round % 2 ? "o" : "i");
    await frames(page, 2);
  }
  presence(userId, "o");
  await frames(page, 4);
  return (((await taskTime()) - before) * 1000) / (rounds + 1);
}

/** What the page holds now, after a garbage collection. */
async function measure(browser, page) {
  const cdp = await page.context().newCDPSession(page);
  await cdp.send("HeapProfiler.collectGarbage");
  await frames(page, 4);
  await cdp.send("Performance.enable");
  const { metrics } = await cdp.send("Performance.getMetrics");
  const metric = (name) => metrics.find((entry) => entry.name === name)?.value;
  const inPage = await page.evaluate(() => ({
    dom: document.getElementsByTagName("*").length,
    rows: document.querySelectorAll(".member-panel .member-row").length,
  }));
  const system = await browser.newBrowserCDPSession();
  const { processInfo } = await system.send("SystemInfo.getProcessInfo");
  let renderer = 0;
  let total = 0;
  let known = true;
  for (const process of processInfo) {
    const mb = await pssMb(process.id);
    if (mb === null) known = false;
    total += mb ?? 0;
    if (process.type === "renderer") renderer += mb ?? 0;
  }
  return {
    dom: inPage.dom,
    memberRows: inPage.rows,
    jsHeapMb: metric("JSHeapUsedSize") / 2 ** 20,
    layoutObjects: metric("LayoutObjects"),
    rendererPssMb: known ? renderer : null,
    browserPssMb: known ? total : null,
  };
}

const out = await mkdtemp(join(tmpdir(), "gelabber-member-list-"));
let browser;
const problems = [];
let files;

async function open(count, viewport = { width: 1280, height: 800 }) {
  const context = await browser.newContext({
    viewport,
    locale: "de-DE",
    serviceWorkers: "block",
  });
  await context.route("**/*", async (route) => {
    const url = new URL(route.request().url());
    if (url.origin !== ORIGIN) return route.abort();
    if (url.pathname.startsWith("/api/")) {
      return route.fulfill({
        contentType: "application/json",
        body: JSON.stringify(answer(url.pathname, count)),
      });
    }
    return route.fulfill({
      path: join(out, files.has(url.pathname) ? url.pathname : "index.html"),
    });
  });
  let gateway;
  await context.routeWebSocket(/\/ws(\?|$)/, (socket) => {
    gateway = socket;
    socket.onMessage((raw) => {
      const frame = JSON.parse(raw);
      if (frame.op !== "s") return;
      socket.send(JSON.stringify({ op: "ok", s: frame.s, c: frame.c, n: 0 }));
      if (frame.s === SERVER && !frame.c)
        socket.send(
          JSON.stringify({
            op: "p",
            s: SERVER,
            snap: [
              { u: ME, st: "o" },
              ...Array.from({ length: count }, (_, n) => ({
                u: member(n).user_id,
                st: "o",
              })),
            ],
          }),
        );
    });
  });
  const presence = (userId, status) =>
    gateway.send(JSON.stringify({ op: "p", s: SERVER, u: userId, st: status }));
  const page = await context.newPage();
  page.setDefaultTimeout(30_000);
  page.on("pageerror", (error) => problems.push(error.message));
  page.on("console", (message) => {
    if (message.type() === "error") problems.push(message.text());
  });
  await page.goto(`${ORIGIN}/s/${SERVER}/c/${TEXT}`);
  // On a phone the list is in a drawer, closed until asked for.
  if (viewport.width <= 1130)
    await page.getByRole("button", { name: /^Mitglieder \(/ }).click();
  // Everyone online, and the observer too.
  await page
    .locator(".member-group-heading", { hasText: `— ${count + 1}` })
    .waitFor()
    .catch((error) => {
      // An answer missing above usually breaks the page: say what it said.
      console.log("Page errors so far:", problems);
      throw error;
    });
  await frames(page, 8);
  return { page, presence, close: () => context.close() };
}

const focusedName = (page) =>
  page.evaluate(() => document.activeElement?.getAttribute("aria-label"));

try {
  await build({
    root: web,
    logLevel: "error",
    build: { outDir: out, emptyOutDir: true },
  });
  files = new Set(
    (await readdir(out, { recursive: true })).map((name) => `/${name}`),
  );
  browser = await chromium.launch({
    executablePath:
      process.env.GELABBER_MEMBER_LIST_BROWSER_EXECUTABLE || undefined,
    headless: true,
    env: { ...process.env, DBUS_SESSION_BUS_ADDRESS: "disabled:" },
  });
  const results = [];
  for (const count of COUNTS) {
    const opened = await open(count);
    const numbers = await measure(browser, opened.page);
    numbers.presenceUpdateMs = await updateCost(
      opened.page,
      opened.presence,
      count > 0 ? member(Math.floor(count / 2)).user_id : ME,
    );
    results.push({ online: count, ...numbers });
    if (process.env.GELABBER_MEMBER_LIST_SHOTS)
      await opened.page.locator(".member-panel").screenshot({
        path: join(
          process.env.GELABBER_MEMBER_LIST_SHOTS,
          `members-${count}.png`,
        ),
      });
    console.log(
      `MEASURE: ${count} online: ${numbers.dom} DOM nodes, ${numbers.memberRows} member rows in the page, JS heap ${numbers.jsHeapMb.toFixed(1)} MB, ${numbers.presenceUpdateMs.toFixed(1)} ms main thread per presence change` +
        (numbers.rendererPssMb === null
          ? ""
          : `, tab ${numbers.rendererPssMb.toFixed(0)} MB, browser ${numbers.browserPssMb.toFixed(0)} MB (PSS)`),
    );
    if (MEASURE_ONLY) {
      await opened.close();
      continue;
    }
    const { page, presence } = opened;
    assert.ok(
      numbers.memberRows < 80,
      `${count} online: ${numbers.memberRows} member rows are in the page`,
    );

    // One Tab stop: from the heading's count to the first member, the
    // observer (sorted first: "Beobachter").
    await page.locator(".member-panel-heading").click();
    await page.keyboard.press("Tab");
    assert.equal(await focusedName(page), "Profil von Beobachter");
    await page.keyboard.press("Tab");
    assert.ok(
      !(await page.evaluate(() =>
        document
          .querySelector(".member-panel")
          ?.contains(document.activeElement),
      )),
      "a second Tab leaves the list",
    );
    await page.keyboard.press("Shift+Tab");
    assert.equal(await focusedName(page), "Profil von Beobachter");

    // ↓ walks past the rows mounted at first.
    const steps = 150;
    for (let step = 0; step < steps; step++)
      await page.keyboard.press("ArrowDown");
    await frames(page);
    const walked = `Profil von Mitglied ${String(steps - 1).padStart(5, "0")}`;
    assert.equal(await focusedName(page), walked);
    const mounted = await page.locator(".member-panel .member-row").count();
    assert.ok(
      mounted < 80,
      `after walking, ${mounted} member rows are mounted`,
    );
    console.log(
      `PASS: ${count} online: ↓ walks ${steps} members down with ${mounted} rows in the page`,
    );

    // Enter opens the profile; closing it hands focus back to the row.
    await page.keyboard.press("Enter");
    const dialog = page.getByRole("dialog", { name: walked });
    await dialog.waitFor();
    await page.keyboard.press("Escape");
    await dialog.waitFor({ state: "detached" });
    await frames(page);
    assert.equal(await focusedName(page), walked);
    console.log(`PASS: ${count} online: Enter opens a profile, Escape returns`);

    // The focused member goes offline: their row moves to the end of the
    // list and leaves the page. Focus goes to the row now in its place.
    presence(member(steps - 1).user_id, "x");
    await page.waitForFunction(
      (name) => document.activeElement?.getAttribute("aria-label") !== name,
      walked,
    );
    await frames(page);
    const after = await focusedName(page);
    assert.equal(
      after,
      `Profil von Mitglied ${String(steps).padStart(5, "0")}`,
      "focus moves to the next member when the focused one goes offline",
    );
    console.log(
      `PASS: ${count} online: focus stays in the list when its member goes offline`,
    );

    // The end of the list: the member who went offline, alone in the
    // offline group, and the last one online above it.
    await page.evaluate(() => {
      const list = document.querySelector(".member-panel-list");
      list.scrollTop = list.scrollHeight;
    });
    await frames(page, 8);
    await page.evaluate(() => {
      const list = document.querySelector(".member-panel-list");
      list.scrollTop = list.scrollHeight;
    });
    await frames(page, 8);
    await page
      .locator(".member-panel")
      .getByRole("button", {
        name: `Profil von Mitglied ${String(count - 1).padStart(5, "0")}`,
      })
      .waitFor();
    await page.locator(".member-group-heading", { hasText: "— 1" }).waitFor();
    const end = await page.evaluate(() => {
      const list = document.querySelector(".member-panel-list");
      const rows = [...list.querySelectorAll("[data-index]")];
      const last = rows.at(-1).getBoundingClientRect();
      const box = list.getBoundingClientRect();
      return { lastBottom: last.bottom, listBottom: box.bottom };
    });
    assert.ok(
      end.lastBottom <= end.listBottom,
      `the last row ends at ${end.lastBottom}px, the list at ${end.listBottom}px`,
    );
    console.log(`PASS: ${count} online: the end of the list scrolls into view`);
    await opened.close();

    // A phone: the list was measured while its drawer was closed. Opened,
    // it starts at its top, where it was, with its rows laid out.
    const phone = await open(count, { width: 390, height: 844 });
    const top = await phone.page.evaluate(() => {
      const list = document.querySelector(".member-panel-list");
      const heading = list.querySelector(".member-group-heading");
      const rows = [...list.querySelectorAll(".member-row")].map((row) =>
        Math.round(row.getBoundingClientRect().top),
      );
      return {
        scrollTop: list.scrollTop,
        heading: Boolean(heading && heading.getBoundingClientRect().height > 0),
        rows: rows.length,
        stacked: rows.every((y, index) => index === 0 || y > rows[index - 1]),
      };
    });
    assert.deepEqual(
      { ...top, rows: top.rows < 80 },
      { scrollTop: 0, heading: true, rows: true, stacked: true },
      `phone drawer: ${JSON.stringify(top)}`,
    );
    console.log(
      `PASS: ${count} online: the phone drawer opens at the list's top with ${top.rows} rows in the page`,
    );
    await phone.close();
  }
  console.log(`RESULT ${JSON.stringify(results)}`);
  const loop = "ResizeObserver loop completed with undelivered notifications.";
  assert.deepEqual(
    problems.filter((problem) => problem !== loop),
    [],
    "the page reported errors",
  );
  console.log(
    `PASS: no page errors, ${browser.browserType().name()} ${browser.version()}`,
  );
} finally {
  await browser?.close();
  await rm(out, { recursive: true, force: true });
}
