// Lazy parts in a real engine, without a server: headless Chromium runs the
// built app from a made-up origin; its files, /api and the gateway are
// answered from here. It checks what the unit tests cannot see:
//  - the first screen loads none of the lazy chunks, and the workspace
//    fetches all of them once the browser is idle;
//  - voice room, a settings area and the server settings render through
//    their parts;
//  - a chunk the server no longer has (as after a deploy) shows an error in
//    place, with no automatic reload, and "Seite neu laden" recovers.
// Run in web/: node scripts/smoke-lazy-parts.mjs
// GELABBER_LAZY_BROWSER_EXECUTABLE names a Chromium other than Playwright's.
/* global process, console, window, URL */
import assert from "node:assert/strict";
import { mkdtemp, readdir, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { chromium } from "playwright";
import { build } from "vite";

const web = fileURLToPath(new URL("..", import.meta.url));
const ORIGIN = "https://lazy.test";
const id = (n) => `00000000-0000-4000-8000-${String(n).padStart(12, "0")}`;
const ME = id(6),
  JONAS = id(7),
  SERVER = id(101),
  TEXT = id(201),
  VOICE = id(202);
const AT = "2026-01-01T00:00:00Z";
const permissions = [
  "send_messages",
  "send_files",
  "join_voice",
  "manage_server",
];
const server = {
  id: SERVER,
  name: "Wohnzimmer",
  owner_id: ME,
  created_at: AT,
  role: "owner",
  permissions,
  member_permissions: permissions,
};
const channel = (cid, name, kind) => ({
  id: cid,
  server_id: SERVER,
  category_id: null,
  name,
  kind,
  created_at: AT,
});
const person = (uid, name) => ({ id: uid, name, avatar_url: null });
const detail = {
  ...server,
  categories: [],
  channels: [
    channel(TEXT, "allgemein", "text"),
    channel(VOICE, "Sofa", "voice"),
  ],
  members: [
    {
      user_id: ME,
      name: "Lena",
      avatar_url: null,
      joined_at: AT,
      role: "owner",
    },
    {
      user_id: JONAS,
      name: "Jonas",
      avatar_url: null,
      joined_at: AT,
      role: "member",
    },
  ],
};
function answer(path) {
  if (path === "/api/auth/session")
    return {
      user: { ...person(ME, "Lena"), email: "lena@example.test" },
      csrf_token: "a".repeat(64),
    };
  if (path === "/api/servers") return [server];
  if (path === `/api/servers/${SERVER}`) return detail;
  if (
    path === "/api/dms" ||
    path === "/api/messages/unread" ||
    path === "/api/notification-settings"
  )
    return [];
  if (path === "/api/notification-privacy") return { hide_content: false };
  if (path === "/api/me/themes")
    return { version: 1, revision: 1, active: "dark", customThemes: [] };
  if (path.endsWith("/messages")) return { messages: [], has_more: false };
  if (path.endsWith("/invites") || path.endsWith("/bans")) return [];
  return {};
}
const out = await mkdtemp(join(tmpdir(), "gelabber-lazy-"));
let browser;
const problems = [];
try {
  await build({
    root: web,
    logLevel: "error",
    build: { outDir: out, emptyOutDir: true },
  });
  const files = new Set(
    (await readdir(out, { recursive: true })).map((n) => `/${n}`),
  );
  browser = await chromium.launch({
    executablePath: process.env.GELABBER_LAZY_BROWSER_EXECUTABLE || undefined,
    headless: true,
  });
  async function open(path, block) {
    const context = await browser.newContext({
      viewport: { width: 1280, height: 800 },
      locale: "de-DE",
      serviceWorkers: "block",
    });
    const requested = [];
    const blocked = { on: !!block };
    await context.route("**/*", (route) => {
      const url = new URL(route.request().url());
      if (url.origin !== ORIGIN) return route.abort();
      if (url.pathname.startsWith("/api/"))
        return route.fulfill({
          contentType: "application/json",
          body: JSON.stringify(answer(url.pathname)),
        });
      requested.push(url.pathname);
      if (blocked.on && block.test(url.pathname))
        return route.fulfill({ status: 404, body: "gone" });
      return route.fulfill({
        path: join(out, files.has(url.pathname) ? url.pathname : "index.html"),
      });
    });
    await context.routeWebSocket("**/ws", (socket) =>
      socket.onMessage((raw) => {
        const f = JSON.parse(raw);
        if (f.op === "s")
          socket.send(JSON.stringify({ op: "ok", s: f.s, c: f.c, n: 0 }));
      }),
    );
    const page = await context.newPage();
    page.setDefaultTimeout(15_000);
    page.on("pageerror", (e) => problems.push(e.message));
    await page.goto(`${ORIGIN}${path}`);
    return { context, page, requested, blocked };
  }
  // 1. Text channel: no lazy chunk on the first screen, all of them once idle.
  {
    const { context, page, requested } = await open(`/s/${SERVER}/c/${TEXT}`);
    await page.locator(".lr-composer textarea").waitFor();
    const early = requested.filter(
      (p) =>
        /VoiceRoom-|SettingsAreaContent-|ServerSettingsPage-|VoiceSettingsPanel-/.test(
          p,
        ) && p.endsWith(".js"),
    );
    assert.deepEqual(early, [], "the first screen loaded lazy chunks");
    await page.waitForTimeout(6000);
    const late = requested.filter(
      (p) =>
        /VoiceRoom-|SettingsAreaContent-|ServerSettingsPage-|VoiceSettingsPanel-/.test(
          p,
        ) && p.endsWith(".js"),
    );
    assert.equal(late.length, 4, `idle preload fetched ${late}`);
    console.log(
      "PASS: idle preload fetched",
      late.map((p) => p.replace(/-.*/, "")).join(", "),
    );
    await context.close();
  }
  // 2. Voice channel, settings area, server settings render through their parts.
  {
    const { context, page } = await open(`/s/${SERVER}/c/${VOICE}`);
    await page.locator(".voice-room").waitFor();
    console.log("PASS: voice channel shows the voice room");
    await page.goto(`${ORIGIN}/settings/darstellung`);
    await page.getByText("Kompakt", { exact: true }).first().waitFor();
    console.log("PASS: settings area renders");
    await page.goto(`${ORIGIN}/s/${SERVER}/settings`);
    await page
      .getByRole("heading", { level: 1, name: /Servereinstellungen/ })
      .waitFor();
    console.log("PASS: server settings render");
    await context.close();
  }
  // 3. A chunk the server no longer has: the error in place, no reload.
  {
    const { context, page, blocked } = await open(
      `/settings/darstellung`,
      /SettingsAreaContent-/,
    );
    await page
      .getByRole("heading", { level: 2, name: "Darstellung" })
      .waitFor();
    await page.evaluate(() => {
      window.__marker = 1;
    });
    await page
      .getByText("Dieser Bereich konnte nicht geladen werden.")
      .waitFor();
    await page.waitForTimeout(1500);
    assert.equal(
      await page.evaluate(() => window.__marker),
      1,
      "the page reloaded by itself",
    );
    assert.ok(
      await page.getByRole("button", { name: "Seite neu laden" }).isVisible(),
    );
    // The sidebar and header still work.
    assert.ok(
      await page
        .getByRole("navigation", { name: "Einstellungsbereiche" })
        .isVisible(),
    );
    blocked.on = false;
    await page.getByRole("button", { name: "Seite neu laden" }).click();
    await page.getByText("Kompakt", { exact: true }).first().waitFor();
    console.log(
      "PASS: a missing chunk shows the error in place, no automatic reload; the reload button recovers",
    );
    await context.close();
  }
  assert.deepEqual(problems, [], "the page reported errors");
  console.log(
    `PASS: no page errors, ${browser.browserType().name()} ${browser.version()}`,
  );
} finally {
  await browser?.close();
  await rm(out, { recursive: true, force: true });
}
