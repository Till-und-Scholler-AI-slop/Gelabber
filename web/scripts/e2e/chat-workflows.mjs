/* global process, document, window, fetch, Event, console, setTimeout, Buffer */
// Local browser acceptance for read state, scoped search and composer drafts.
import assert from "node:assert/strict";
import { mkdir, writeFile } from "node:fs/promises";
import { randomBytes } from "node:crypto";
import { chromium } from "playwright";
import { api, until, safeTarget } from "./harness.mjs";
const base = safeTarget(
  process.env.GELABBER_SMOKE_URL ?? "http://127.0.0.1:5174",
);
const output =
  process.env.GELABBER_CHAT_REPORT ?? "/tmp/gelabber-chat-workflows";
await mkdir(output, { recursive: true });
// Another Chromium build than Playwright's own, e.g. where its download is
// blocked (like GELABBER_PWA_BROWSER_EXECUTABLE in smoke-pwa.mjs).
const browser = await chromium.launch(
  process.env.GELABBER_CHAT_BROWSER_EXECUTABLE
    ? { executablePath: process.env.GELABBER_CHAT_BROWSER_EXECUTABLE }
    : {},
);
const contexts = [];
const suffix = randomBytes(6).toString("hex");
const password = randomBytes(20).toString("base64url");
const checks = [];
let server, owner;
const check = async (name, run) => {
  await run();
  checks.push(name);
  console.log(`PASS ${name}`);
};
async function actor(label, credentials) {
  // The checks find controls by their German names.
  const context = await browser.newContext({ locale: "de-DE" });
  contexts.push(context);
  const page = await context.newPage();
  page.setDefaultTimeout(12_000);
  const who = {
    context,
    page,
    email: credentials?.email ?? `chat-${label}-${suffix}@example.test`,
    password,
  };
  await page.goto(`${base}/${credentials ? "login" : "register"}`);
  if (!credentials)
    await page.getByLabel("Name", { exact: true }).fill(`Chat ${label}`);
  await page.getByLabel("E-Mail-Adresse").fill(who.email);
  await page.getByLabel("Passwort", { exact: true }).fill(password);
  await page
    .getByRole("button", {
      name: credentials ? "Anmelden" : "Registrieren",
      exact: true,
    })
    .click();
  await page.waitForURL((url) => !/(login|register)$/.test(url.pathname));
  who.id = (await api(who, "/auth/session")).body.user.id;
  return who;
}
const channelState = async (who, id) =>
  (await api(who, "/messages/unread")).body.find(
    (row) => row.channel_id === id,
  );
// A sidebar row whose accessible name says it is unread, dot or count.
const unreadRow = (who, path) =>
  who.page
    .locator(`.channel-sidebar a[href="${path}"]`)
    .and(who.page.getByRole("link", { name: /ungelesen/i }));
const send = async (who, id, content) => {
  const r = await api(who, `/channels/${id}/messages`, "POST", { content });
  assert.equal(r.status, 201);
  return r.body;
};
try {
  owner = await actor("owner");
  const member = await actor("member");
  const created = await api(owner, "/servers", "POST", {
    name: `Chat ${suffix}`,
  });
  assert.equal(created.status, 201);
  server = created.body.id;
  const channel = created.body.channels.find((c) => c.kind === "text").id;
  const second = (
    await api(owner, `/servers/${server}/channels`, "POST", {
      name: "draft-room",
      kind: "text",
    })
  ).body.id;
  const invite = (await api(owner, `/servers/${server}/invites`, "POST", {}))
    .body.code;
  assert.equal(
    (await api(member, `/invites/${invite}/join`, "POST")).status,
    200,
  );
  const path = `/s/${server}/c/${channel}`,
    other = `/s/${server}/c/${second}`;
  await member.page.goto(`${base}${other}`);
  await check("unopened channel badge excludes own posts", async () => {
    for (let i = 0; i < 36; i++)
      await send(owner, channel, `history ${i} searchneedle`);
    await send(member, channel, "own excluded");
    assert.equal((await channelState(member, channel)).unread_count, 36);
    // Plain messages in a server channel show a dot; the row's name says
    // "ungelesen" either way (unreadLabel in notifyLevel.ts).
    await unreadRow(member, path).waitFor();
  });
  await check(
    "read follows visible latest, preserves history and hidden unread",
    async () => {
      await member.page
        .locator(`.channel-sidebar a[href="${path}"]`)
        .first()
        .click();
      await member.page.bringToFront();
      await until(
        () => channelState(member, channel),
        (row) => row.unread_count === 0,
        "initial-read",
      );
      const log = member.page.getByRole("log", { name: "Nachrichten" });
      await log.evaluate((el) => {
        el.scrollTop = 0;
        el.dispatchEvent(new Event("scroll"));
      });
      await send(owner, channel, "history must stay unread");
      await until(
        () => channelState(member, channel),
        (row) => row.unread_count === 1,
        "history-count",
      );
      await new Promise((r) => setTimeout(r, 900));
      assert.equal((await channelState(member, channel)).unread_count, 1);
      // From the top, the jump lands at the end the virtual list estimates;
      // rows measured there can move the end on. Scroll until the list holds
      // at its end, as a reader keeps scrolling, then the newest is read.
      const atEnd = () =>
        log.evaluate(async (el) => {
          el.scrollTop = el.scrollHeight;
          const state = () => [el.scrollHeight, el.clientHeight, el.scrollTop];
          let still = 0;
          let last = state();
          while (still < 5) {
            await new Promise(requestAnimationFrame);
            const now = state();
            still = now.every((v, i) => v === last[i]) ? still + 1 : 0;
            last = now;
          }
          const box = el.getBoundingClientRect();
          const newest = [...el.querySelectorAll("[data-index]")].find((row) =>
            row.textContent.includes("history must stay unread"),
          );
          const rect = newest?.getBoundingClientRect();
          return {
            gap: el.scrollHeight - el.scrollTop - el.clientHeight,
            newestDrawn: !!newest,
            newestOnScreen:
              !!rect && rect.top >= box.top && rect.bottom <= box.bottom + 1,
          };
        });
      await until(atEnd, (state) => state.gap <= 1, "scroll-to-end");
      // Its last sample says where the list stood: at the end with the newest
      // on screen and still unread is the app's fault.
      await until(
        async () => ({
          ...(await channelState(member, channel)),
          list: await atEnd(),
        }),
        (row) => row.unread_count === 0,
        "scroll-read",
      );
      // Deterministic visibility seam; actual device/tab lifecycle acceptance is separate.
      await member.page.evaluate(() =>
        Object.defineProperty(document, "visibilityState", {
          configurable: true,
          get: () => "hidden",
        }),
      );
      await send(owner, channel, "hidden must stay unread");
      await until(
        () => channelState(member, channel),
        (row) => row.unread_count === 1,
        "hidden-count",
      );
      await new Promise((r) => setTimeout(r, 900));
      assert.equal((await channelState(member, channel)).unread_count, 1);
      await member.page.evaluate(() => {
        delete document.visibilityState;
        document.dispatchEvent(new Event("visibilitychange"));
      });
      await until(
        () => channelState(member, channel),
        (row) => row.unread_count === 0,
        "visible-read",
      );
    },
  );
  await check(
    "search shows scoped results and stays outside read tracking",
    async () => {
      await member.page
        .getByRole("searchbox", { name: "In dieser Unterhaltung suchen" })
        .fill("searchneedle");
      const results = member.page.getByRole("region", {
        name: "Nachrichten suchen",
      });
      await until(
        () => results.locator("article").count(),
        (count) => count === 25,
        "search-first-page",
      );
      await member.page.getByRole("button", { name: "Ältere Treffer" }).click();
      await until(
        () => results.locator("article").count(),
        (count) => count === 36,
        "search-older-page",
      );
      await send(owner, channel, "arrived while searching");
      await until(
        () => channelState(member, channel),
        (row) => row.unread_count === 1,
        "search-unread",
      );
      await new Promise((r) => setTimeout(r, 900));
      assert.equal((await channelState(member, channel)).unread_count, 1);
      await member.page.screenshot({ path: `${output}/search-desktop.png` });
      await member.page
        .getByRole("button", { name: "Zurück zum Chat" })
        .click();
      await until(
        () => channelState(member, channel),
        (row) => row.unread_count === 0,
        "search-close-read",
      );
    },
  );
  await check(
    "text survives reload, file survives navigation, logout releases both",
    async () => {
      await member.page
        .locator(`.channel-sidebar a[href="${other}"]`)
        .first()
        .click();
      const composer = member.page.locator("form textarea").last();
      await composer.fill("draft across navigation and reload");
      // The first of the paperclip's inputs takes every type (#574).
      await member.page
        .locator('input[type="file"]')
        .first()
        .setInputFiles({
          name: "draft.txt",
          mimeType: "text/plain",
          buffer: Buffer.from("unsent file"),
        });
      await member.page
        .locator(`.channel-sidebar a[href="${path}"]`)
        .first()
        .click();
      await member.page
        .locator(`.channel-sidebar a[href="${other}"]`)
        .first()
        .click();
      assert.equal(
        await member.page.locator("form textarea").last().inputValue(),
        "draft across navigation and reload",
      );
      await member.page.getByText("draft.txt", { exact: true }).waitFor();
      await member.page.reload();
      assert.equal(
        await member.page.locator("form textarea").last().inputValue(),
        "draft across navigation and reload",
      );
      assert.equal(
        await member.page.getByText("draft.txt", { exact: true }).count(),
        0,
      );
      await member.page.setViewportSize({ width: 390, height: 844 });
      await member.page.screenshot({ path: `${output}/draft-mobile.png` });
      await member.page.evaluate(async () => {
        const session = await fetch("/api/auth/session").then((r) => r.json());
        await fetch("/api/auth/logout", {
          method: "POST",
          headers: { "X-CSRF-Token": session.csrf_token },
        });
        window.dispatchEvent(new Event("focus"));
      });
      // The focus check finds the session gone and releases the drafts. A
      // page load before that would start with the old drafts still stored.
      await member.page.waitForFunction(
        () =>
          !Object.keys(window.localStorage).some((key) =>
            key.startsWith("gelabber:chat-draft:"),
          ),
      );
      await member.page.goto(`${base}/login`);
      await member.page.getByLabel("E-Mail-Adresse").fill(member.email);
      await member.page.getByLabel("Passwort", { exact: true }).fill(password);
      await member.page
        .getByRole("button", { name: "Anmelden", exact: true })
        .click();
      await member.page.waitForURL((url) => !url.pathname.endsWith("login"));
      await member.page.goto(`${base}${other}`);
      assert.equal(
        await member.page.locator("form textarea").last().inputValue(),
        "",
      );
    },
  );
  await check("another device read converges badges on focus", async () => {
    const device = await actor("device", member);
    await member.page.goto(`${base}${other}`);
    await send(owner, channel, "another device reads");
    await until(
      () => channelState(member, channel),
      (row) => row.unread_count === 1,
      "device-unread",
    );
    await device.page.goto(`${base}${path}`);
    await device.page.bringToFront();
    await until(
      () => channelState(device, channel),
      (row) => row.unread_count === 0,
      "device-read",
    );
    await member.page.bringToFront();
    await member.page.evaluate(() => window.dispatchEvent(new Event("focus")));
    await until(
      () => unreadRow(member, path).count(),
      (count) => count === 0,
      "badge-device-convergence",
      // The focus refresh waits until the previous snapshot is 10 s old.
      15_000,
    );
  });
  await check(
    "file note closes with ×, Escape, typing and its time (GEL-8)",
    async () => {
      const page = member.page;
      await page.goto(`${base}${path}`);
      const composer = page.locator("form textarea").last();
      const note = page.locator(".lr-composer-file-error");
      const refuse = () =>
        page
          .locator('input[type="file"]')
          .first()
          .setInputFiles({
            name: "setup.exe",
            mimeType: "application/x-msdownload",
            buffer: Buffer.from("MZ"),
          });
      const text = "setup.exe: Dieser Dateityp ist nicht erlaubt.";
      await refuse();
      await note.getByText(text, { exact: true }).waitFor();
      await page.screenshot({ path: `${output}/file-note.png` });
      await page.getByRole("button", { name: "Hinweis schließen" }).click();
      await note.waitFor({ state: "detached" });
      await refuse();
      await note.waitFor();
      await composer.focus();
      await page.keyboard.press("Escape");
      await note.waitFor({ state: "detached" });
      await refuse();
      await note.waitFor();
      await composer.pressSequentially("H");
      await note.waitFor({ state: "detached" });
      await composer.fill("");
      // A file that is fine after one left out: no note stays.
      await refuse();
      await note.waitFor();
      await page
        .locator('input[type="file"]')
        .first()
        .setInputFiles({
          name: "liste.txt",
          mimeType: "text/plain",
          buffer: Buffer.from("Milch"),
        });
      await note.waitFor({ state: "detached" });
      await page.getByRole("button", { name: "liste.txt entfernen" }).click();
      // Held while the pointer is on it, gone 8 s after it leaves.
      await refuse();
      await note.hover();
      await page.waitForTimeout(9_000);
      assert.equal(await note.count(), 1);
      await page.mouse.move(0, 0);
      await note.waitFor({ state: "detached", timeout: 9_500 });
    },
  );
  await check("DM drafts and search use the same scoped workflow", async () => {
    const dm = (await api(owner, "/dms", "POST", { user_id: member.id })).body
      .id;
    // Not friends: the DM is a message request (#563), which the unread
    // list leaves out until it is accepted.
    assert.equal((await api(member, `/dms/${dm}/accept`, "POST")).status, 200);
    await send(owner, dm, "dm searchneedle");
    await member.page.goto(`${base}/d/${dm}`);
    await member.page.bringToFront();
    await until(
      () => channelState(member, dm),
      (row) => row.unread_count === 0,
      "dm-read",
    );
    await member.page.locator("form textarea").last().fill("dm draft");
    await member.page.reload();
    assert.equal(
      await member.page.locator("form textarea").last().inputValue(),
      "dm draft",
    );
    // On a phone the field is an icon until it is tapped (#561).
    await member.page.locator(".lr-header-search").click();
    await member.page
      .getByRole("searchbox", { name: "In dieser Unterhaltung suchen" })
      .fill("searchneedle");
    await member.page
      .getByRole("region", { name: "Nachrichten suchen" })
      .getByText("dm searchneedle", { exact: true })
      .waitFor();
    await member.page.screenshot({ path: `${output}/dm-search-mobile.png` });
  });
} finally {
  if (server && owner)
    await api(owner, `/servers/${server}`, "DELETE").catch(() => {});
  for (const context of contexts) await context.close();
  await browser.close();
  await writeFile(
    `${output}/report.json`,
    JSON.stringify(
      {
        checks,
        browser: "Chromium",
        mobileViewport: "390x844",
        physicalDeviceAcceptance: false,
      },
      null,
      2,
    ),
  );
}
