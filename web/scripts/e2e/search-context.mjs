/* global process, document, sessionStorage, Event, console, setTimeout, Buffer */
// Local browser integration for scoped search navigation; no physical-device claim.
import assert from "node:assert/strict";
import { mkdir, writeFile } from "node:fs/promises";
import { randomBytes } from "node:crypto";
import { chromium } from "playwright";
import { api, until, safeTarget } from "./harness.mjs";
const base = safeTarget(
  process.env.GELABBER_SMOKE_URL ?? "http://127.0.0.1:5174",
);
const output =
  process.env.GELABBER_SEARCH_REPORT ?? "/tmp/gelabber-search-context";
await mkdir(output, { recursive: true });
const browser = await chromium.launch();
const contexts = [];
const suffix = randomBytes(6).toString("hex");
const password = randomBytes(20).toString("base64url");
const checks = [];
let server, owner, memberDebug;
const check = async (name, run) => {
  try {
    await run();
  } catch (error) {
    await memberDebug?.screenshot({ path: `${output}/failure.png` });
    if (memberDebug)
      await writeFile(
        `${output}/debug.json`,
        JSON.stringify(
          await memberDebug.evaluate(async () => {
            const { queryClient } = await import("/src/queryClient.ts");
            const { getGateway } = await import("/src/ws/client.ts");
            return {
              text: document.body.innerText,
              cursors: [...getGateway().topicCursorsSnapshot],
              queries: queryClient
                .getQueryCache()
                .getAll()
                .filter((q) =>
                  ["message-context", "message-search"].includes(
                    String(q.queryKey[3]),
                  ),
                )
                .map((q) => ({
                  key: q.queryKey,
                  status: q.state.status,
                  fetch: q.state.fetchStatus,
                  error: q.state.error,
                  data: q.state.data,
                })),
            };
          }),
          null,
          2,
        ),
      );
    throw error;
  }
  checks.push(name);
  console.log(`PASS ${name}`);
};
async function actor(label, credentials) {
  const context = await browser.newContext({ hasTouch: true });
  contexts.push(context);
  const page = await context.newPage();
  const frames = [];
  page.on("websocket", (ws) =>
    ws.on("framereceived", (event) => {
      try {
        frames.push(JSON.parse(String(event.payload)));
      } catch {
        /* heartbeat data */
      }
    }),
  );
  page.setDefaultTimeout(12_000);
  const who = {
    context,
    page,
    frames,
    email: credentials?.email ?? `chat-${label}-${suffix}@example.test`,
    password,
  };
  await page.goto(`${base}/${credentials ? "login" : "register"}`);
  if (!credentials)
    await page
      .getByLabel("Name", { exact: true })
      .waitFor()
      .catch(async (error) => {
        await page.screenshot({
          path: `${output}/fixture-${label}-failure.png`,
        });
        await writeFile(
          `${output}/fixture-${label}-failure.txt`,
          await page.locator("body").innerText(),
        );
        throw error;
      });
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
  for (let attempt = 0; attempt < 3; attempt++) {
    const r = await api(who, `/channels/${id}/messages`, "POST", { content });
    if (r.status === 429 && attempt < 2) {
      const delay = Math.min(
        60_000,
        Math.max(1000, (r.body.retry_after ?? 1) * 1000 + 100),
      );
      await new Promise((resolve) => setTimeout(resolve, delay));
      continue;
    }
    assert.equal(r.status, 201);
    return r.body;
  }
  throw new Error("fixture message retry exhausted");
};
try {
  owner = await actor("owner");
  const member = await actor("member");
  memberDebug = member.page;
  const replacement = await actor("replacement");
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

  const needle = "contextneedle";
  const target = await send(owner, channel, `${needle} original`);
  for (let i = 0; i < 55; i++) await send(owner, channel, `filler ${i}`);
  const latest = (await api(member, `/channels/${channel}/messages`)).body
    .messages;
  assert.ok(
    !latest.some((row) => row.id === target.id),
    "fixture target must precede latest page",
  );
  await member.page.evaluate(() =>
    Object.defineProperty(document, "visibilityState", {
      configurable: true,
      get: () => "hidden",
    }),
  );
  await member.page.locator(`a[href="${path}"]`).first().click();
  const log = member.page.getByRole("log", { name: "Nachrichten" });
  await log.waitFor();
  await log.evaluate((el) => {
    el.scrollTop = Math.max(0, el.scrollHeight - el.clientHeight - 220);
    el.dispatchEvent(new Event("scroll"));
  });
  await new Promise((r) => setTimeout(r, 500));
  const composer = member.page.locator("form textarea").last();
  await composer.fill("unsent preserved draft");
  await member.page.locator('input[type="file"]').setInputFiles({
    name: "context-draft.txt",
    mimeType: "text/plain",
    buffer: Buffer.from("unsent"),
  });
  const beforeScroll = await log.evaluate((el) => el.scrollTop);
  const beforeUnread = (await channelState(member, channel)).unread_count;
  const search = async (value) => {
    await member.page
      .getByRole("button", { name: "Nachrichten suchen", exact: true })
      .click();
    await member.page.getByRole("searchbox").fill(value);
    await member.page
      .getByRole("button", { name: "Suchen", exact: true })
      .click();
  };
  const contextRegion = () =>
    member.page.getByRole("region", { name: "Nachrichtenkontext" });
  let failed = true;
  const contextUrl = `**/api/channels/${channel}/messages/${target.id}/context`;
  await member.page.route(contextUrl, async (route) => {
    if (failed)
      await route.fulfill({
        status: 500,
        contentType: "application/json",
        body: JSON.stringify({
          error: "internal",
          message: "local retry seam",
          fields: {},
        }),
      });
    else await route.continue();
  });
  await check(
    "keyboard opens bounded old-message context and retry focuses target",
    async () => {
      await search(needle);
      const hit = member.page.getByRole("button", {
        name: `Zur Nachricht von Chat owner: ${needle} original`,
        exact: true,
      });
      await hit.focus();
      await member.page.keyboard.press("Enter");
      await contextRegion().getByRole("alert").waitFor();
      failed = false;
      await contextRegion()
        .getByRole("button", { name: "Erneut laden" })
        .click();
      const focused = contextRegion().locator(
        `[data-message-id="${target.id}"]`,
      );
      await focused.waitFor();
      await until(
        () => focused.evaluate((el) => document.activeElement === el),
        Boolean,
        "target-keyboard-focus",
      );
      assert.ok((await contextRegion().locator("article").count()) <= 61);
      await member.page.bringToFront();
      await member.page.evaluate(() => {
        delete document.visibilityState;
        document.dispatchEvent(new Event("visibilitychange"));
      });
      await new Promise((r) => setTimeout(r, 900));
      assert.equal(
        (await channelState(member, channel)).unread_count,
        beforeUnread,
      );
      assert.equal(await composer.inputValue(), "unsent preserved draft");
      await member.page
        .getByText("context-draft.txt", { exact: true })
        .waitFor();
      await member.page.screenshot({ path: `${output}/context-desktop.png` });
    },
  );
  await check(
    "touch opens the same old target at a phone viewport",
    async () => {
      await member.page.setViewportSize({ width: 390, height: 844 });
      await contextRegion()
        .getByRole("button", { name: "Zurück zu den Treffern" })
        .tap();
      await member.page
        .getByRole("button", {
          name: `Zur Nachricht von Chat owner: ${needle} original`,
          exact: true,
        })
        .tap();
      const focused = contextRegion().locator(
        `[data-message-id="${target.id}"]`,
      );
      await focused.waitFor();
      await until(
        () => focused.evaluate((el) => document.activeElement === el),
        Boolean,
        "touch-target-focus",
      );
      assert.equal(
        (await channelState(member, channel)).unread_count,
        beforeUnread,
      );
      await member.page.screenshot({
        path: `${output}/touch-context-mobile.png`,
      });
    },
  );
  await check(
    "edit and delete converge in open context without marking latest read",
    async () => {
      assert.equal(
        (
          await api(owner, `/messages/${target.id}`, "PATCH", {
            content: `${needle} edited`,
          })
        ).status,
        200,
      );
      await contextRegion()
        .getByText(`${needle} edited`, { exact: false })
        .waitFor();
      await member.page.setViewportSize({ width: 390, height: 844 });
      await member.page.screenshot({ path: `${output}/context-mobile.png` });
      assert.equal(
        (await api(owner, `/messages/${target.id}`, "DELETE")).status,
        204,
      );
      await contextRegion()
        .getByText(
          "Diese Nachricht ist nicht mehr verfügbar oder du hast keinen Zugriff mehr.",
          { exact: true },
        )
        .waitFor();
      assert.equal(await contextRegion().locator("article").count(), 0);
      await contextRegion()
        .getByRole("button", { name: "Erneut laden" })
        .click();
      await contextRegion()
        .getByText(
          "Diese Nachricht ist nicht mehr verfügbar oder du hast keinen Zugriff mehr.",
          { exact: true },
        )
        .waitFor();
      await contextRegion()
        .getByRole("button", { name: "Zurück zu den Treffern" })
        .click();
      await member.page
        .getByText("Keine Nachrichten gefunden.", { exact: true })
        .waitFor();
      await member.page
        .getByRole("button", { name: "Zurück zum Chat", exact: true })
        .click();
      await member.page.setViewportSize({ width: 1280, height: 720 });
      assert.equal(await composer.inputValue(), "unsent preserved draft");
      await member.page
        .getByText("context-draft.txt", { exact: true })
        .waitFor();
      const afterScroll = await log.evaluate((el) => el.scrollTop);
      assert.ok(
        Math.abs(afterScroll - beforeScroll) < 100,
        `history scroll was changed ${beforeScroll}->${afterScroll}`,
      );
      await new Promise((r) => setTimeout(r, 900));
      assert.equal(
        (await channelState(member, channel)).unread_count,
        beforeUnread - 1,
      );
    },
  );
  const cancelTarget = await send(
    owner,
    channel,
    "cancelneedle private source",
  );
  const blockedUrl = `**/api/channels/${channel}/messages/${cancelTarget.id}/context`;
  const beginBlocked = async () => {
    let release;
    const gate = new Promise((r) => {
      release = r;
    });
    let started;
    const requestStarted = new Promise((r) => {
      started = r;
    });
    let routeFinished;
    const finished = new Promise((r) => {
      routeFinished = r;
    });
    const handler = async (route) => {
      started();
      await gate;
      try {
        await route.continue();
      } catch {
        /* Account/chat cancellation may abort the owned request. */
      }
      routeFinished();
    };
    await member.page.route(blockedUrl, handler);
    await search("cancelneedle");
    await member.page
      .getByRole("button", {
        name: /Zur Nachricht von Chat owner: cancelneedle/,
      })
      .click();
    await requestStarted;
    return async () => {
      release();
      await finished;
      await member.page.unroute(blockedUrl, handler);
    };
  };
  await check(
    "changing chat cancels private context and keeps drafts",
    async () => {
      const release = await beginBlocked();
      const canceled = member.page.waitForEvent("requestfailed", {
        predicate: (r) => r.url().endsWith(`/${cancelTarget.id}/context`),
      });
      await member.page.locator(`a[href="${other}"]`).first().click();
      await canceled;
      await release();
      assert.equal(await contextRegion().count(), 0);
      assert.equal(
        await member.page
          .getByText("cancelneedle private source", { exact: true })
          .count(),
        0,
      );
      await member.page.locator(`a[href="${path}"]`).first().click();
      assert.equal(await composer.inputValue(), "unsent preserved draft");
      await member.page
        .getByText("context-draft.txt", { exact: true })
        .waitFor();
    },
  );
  await check(
    "account replacement cancels the private request and releases drafts",
    async () => {
      const release = await beginBlocked();
      const canceled = member.page.waitForEvent("requestfailed", {
        predicate: (r) => r.url().endsWith(`/${cancelTarget.id}/context`),
      });
      await member.page.locator('summary[aria-label="Benutzermenü"]').click();
      await member.page
        .getByRole("button", { name: "Abmelden", exact: true })
        .click();
      await member.page.waitForURL((url) => url.pathname === "/login");
      await canceled;
      await member.page.getByLabel("E-Mail-Adresse").fill(replacement.email);
      await member.page.getByLabel("Passwort", { exact: true }).fill(password);
      await member.page
        .getByRole("button", { name: "Anmelden", exact: true })
        .click();
      await member.page.waitForURL((url) => url.pathname !== "/login");
      await release();
      assert.equal(
        (await api(member, "/auth/session")).body.user.id,
        replacement.id,
      );
      assert.equal(await contextRegion().count(), 0);
      assert.equal(
        await member.page
          .getByText("cancelneedle private source", { exact: true })
          .count(),
        0,
      );
      const storage = await member.page.evaluate(() =>
        Object.keys(sessionStorage).filter((key) => key.includes("draft")),
      );
      assert.equal(storage.length, 0);
    },
  );
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
        visibilityLifecycle: "deterministic browser seam; not OS lifecycle",
      },
      null,
      2,
    ),
  );
}
