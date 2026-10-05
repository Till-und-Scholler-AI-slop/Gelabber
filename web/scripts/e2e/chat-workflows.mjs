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
const browser = await chromium.launch();
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
  const context = await browser.newContext();
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
    await member.page
      .locator(`a[href="${path}"] [aria-label="36 ungelesene Nachrichten"]`)
      .waitFor();
  });
  await check(
    "read follows visible latest, preserves history and hidden unread",
    async () => {
      await member.page.locator(`a[href="${path}"]`).first().click();
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
      await log.evaluate((el) => {
        el.scrollTop = el.scrollHeight;
        el.dispatchEvent(new Event("scroll"));
      });
      await until(
        () => channelState(member, channel),
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
        .getByRole("button", { name: "Nachrichten suchen", exact: true })
        .click();
      await member.page.getByRole("searchbox").fill("searchneedle");
      await member.page
        .getByRole("button", { name: "Suchen", exact: true })
        .click();
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
      await member.page.locator(`a[href="${other}"]`).first().click();
      const composer = member.page.locator("form textarea").last();
      await composer.fill("draft across navigation and reload");
      await member.page.locator('input[type="file"]').setInputFiles({
        name: "draft.txt",
        mimeType: "text/plain",
        buffer: Buffer.from("unsent file"),
      });
      await member.page.locator(`a[href="${path}"]`).first().click();
      await member.page.locator(`a[href="${other}"]`).first().click();
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
      () =>
        member.page
          .locator(`a[href="${path}"] [aria-label*="ungelesene Nachrichten"]`)
          .count(),
      (count) => count === 0,
      "badge-device-convergence",
    );
  });
  await check("DM drafts and search use the same scoped workflow", async () => {
    const dm = (await api(owner, "/dms", "POST", { user_id: member.id })).body
      .id;
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
    await member.page
      .getByRole("button", { name: "Nachrichten suchen", exact: true })
      .click();
    await member.page.getByRole("searchbox").fill("searchneedle");
    await member.page
      .getByRole("button", { name: "Suchen", exact: true })
      .click();
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
