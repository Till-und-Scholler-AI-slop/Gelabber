/* global process, document, window, console, Event, URL */
// Local browser integration: real API/session/gateway, deterministic OS and visibility seams.
// This does not grant an OS permission or validate physical-device delivery.
import assert from "node:assert/strict";
import { mkdir, readFile, writeFile } from "node:fs/promises";
import { execFileSync } from "node:child_process";
import { createHash, randomBytes } from "node:crypto";
import { chromium } from "playwright";
import { api, until, safeTarget, safeError } from "./harness.mjs";

const base = safeTarget(
  process.env.GELABBER_SMOKE_URL ?? "http://127.0.0.1:5174",
);
const output =
  process.env.GELABBER_RECOVERY_REPORT ?? "/tmp/gelabber-webapp-recovery";
await mkdir(output, { recursive: true });
const browser = await chromium.launch();
const contexts = [];
const suffix = randomBytes(6).toString("hex");
const password = randomBytes(24).toString("base64url");
const checks = [];
const provenance = {
  webSource: execFileSync("git", ["rev-parse", "HEAD"], {
    encoding: "utf8",
  }).trim(),
  sourceFileSha256: createHash("sha256")
    .update(await readFile(new URL(import.meta.url)))
    .digest("hex"),
  apiSource:
    process.env.GELABBER_E2E_API_SHA ??
    "operator-supplied local API; revision unverified",
  at: new Date().toISOString(),
};
let server, owner;
let currentCase = "local fixture setup";

const check = async (name, run) => {
  currentCase = name;
  await run();
  checks.push(name);
  console.log(`PASS ${name}`);
};
async function actor(label) {
  const context = await browser.newContext();
  contexts.push(context);
  await context.addInitScript(() => {
    const seam = {
      permission: "default",
      requested: 0,
      reject: false,
      notices: [],
    };
    class MockNotification {
      static get permission() {
        return seam.permission;
      }
      static async requestPermission() {
        seam.requested++;
        if (seam.reject) throw new Error("mock-permission-rejection");
        seam.permission = "granted";
        return seam.permission;
      }
      constructor(title, options) {
        this.title = title;
        this.options = options;
        this.onclick = null;
        this.closed = false;
        seam.notices.push(this);
      }
      close() {
        this.closed = true;
      }
    }
    seam.MockNotification = MockNotification;
    window.__notificationSeam = seam;
    window.Notification = MockNotification;
  });
  const page = await context.newPage();
  page.setDefaultTimeout(12_000);
  const who = {
    context,
    page,
    email: `recovery-${label}-${suffix}@example.test`,
    sockets: [],
    events: [],
  };
  page.on("websocket", (socket) => {
    const stream = { closed: false, frames: [] };
    who.sockets.push(stream);
    socket.on("close", () => {
      stream.closed = true;
    });
    socket.on("framereceived", ({ payload }) => {
      try {
        const frame = JSON.parse(String(payload));
        stream.frames.push(frame);
        if (frame.op === "e") who.events.push(frame);
      } catch {
        /* Non-JSON frames are outside this chat assertion. */
      }
    });
  });
  await page.goto(`${base}/register`);
  await page.getByLabel("Name", { exact: true }).fill(`Recovery ${label}`);
  await page.getByLabel("E-Mail-Adresse").fill(who.email);
  await page.getByLabel("Passwort", { exact: true }).fill(password);
  await page.getByRole("button", { name: "Registrieren", exact: true }).click();
  await page.waitForURL((url) => url.pathname !== "/register");
  who.id = (await api(who, "/auth/session")).body.user.id;
  return who;
}
const liveTopic = async (who, channel) =>
  until(
    () => who.sockets.filter((socket) => !socket.closed).at(-1)?.frames ?? [],
    (frames) =>
      frames.some((frame) => frame.op === "ok" && frame.c === channel),
    "native-gateway-topic-ready",
  );
const renderTurn = (who) =>
  who.page.evaluate(
    () =>
      new Promise((resolve) =>
        window.requestAnimationFrame(() =>
          window.requestAnimationFrame(resolve),
        ),
      ),
  );
const delivered = async (who, message) => {
  await until(
    () =>
      who.events.some((event) => event.t === "c" && event.d?.id === message.id),
    Boolean,
    "native-gateway-created-delivery",
  );
  // Native receipt can precede the page listener; let its synchronous delivery
  // and the rendered consumers finish before asserting absence of a notice.
  await renderTurn(who);
};
const send = async (who, channel, content) => {
  const response = await api(who, `/channels/${channel}/messages`, "POST", {
    content,
  });
  assert.equal(response.status, 201, "fixture-message-create");
  return response.body;
};
const notifications = (who) =>
  who.page.evaluate(() =>
    window.__notificationSeam.notices.map((notice) => ({
      title: notice.title,
      options: notice.options,
      closed: notice.closed,
    })),
  );
async function setHidden(who, hidden) {
  await who.page.evaluate((value) => {
    Object.defineProperty(document, "hidden", {
      configurable: true,
      get: () => value,
    });
    Object.defineProperty(document, "visibilityState", {
      configurable: true,
      get: () => (value ? "hidden" : "visible"),
    });
    document.dispatchEvent(new Event("visibilitychange"));
  }, hidden);
}
async function settings(who, { toast, desktop }) {
  // Keep notification instances alive through actual client-side navigation.
  const link = who.page.locator('a[href="/settings"]').first();
  await link.click();
  await who.page
    .getByRole("button", { name: "Benachrichtigungen", exact: true })
    .click();
  await who.page.locator("#message-toasts").setChecked(toast);
  await who.page.locator("#desktop-notify").setChecked(desktop);
}
async function goBack(who) {
  await who.page.getByRole("button", { name: "Zurück", exact: true }).click();
}
async function login(who, email, secret = password) {
  await who.page.getByLabel("E-Mail-Adresse").fill(email);
  await who.page.getByLabel("Passwort", { exact: true }).fill(secret);
  await who.page.getByRole("button", { name: "Anmelden", exact: true }).click();
}
const fail = (route) =>
  route.fulfill({
    status: 500,
    contentType: "application/json",
    body: JSON.stringify({
      error: "internal",
      message: "local injected failure",
    }),
  });
try {
  owner = await actor("owner");
  const member = await actor("member");
  const replacement = await actor("replacement");
  const created = await api(owner, "/servers", "POST", {
    name: `Recovery ${suffix}`,
  });
  assert.equal(created.status, 201, "fixture-server-create");
  server = created.body.id;
  const channel = created.body.channels.find((row) => row.kind === "text").id;
  const secondResponse = await api(
    owner,
    `/servers/${server}/channels`,
    "POST",
    { name: "notification-room", kind: "text" },
  );
  assert.equal(secondResponse.status, 201, "fixture-channel-create");
  const second = secondResponse.body.id;
  const invite = (await api(owner, `/servers/${server}/invites`, "POST", {}))
    .body.code;
  assert.equal(
    (await api(member, `/invites/${invite}/join`, "POST")).status,
    200,
    "fixture-member-join",
  );
  const path = `/s/${server}/c/${channel}`;
  const other = `/s/${server}/c/${second}`;

  await check(
    "bootstrap API500 preserves deep link and Retry restores real session",
    async () => {
      let blocked = 0;
      const handler = async (route) => {
        blocked++;
        await fail(route);
      };
      await member.page.route("**/api/auth/session", handler);
      await member.page.goto(`${base}${path}`);
      await member.page
        .getByRole("button", { name: "Sitzung erneut prüfen", exact: true })
        .waitFor();
      assert.equal(
        new URL(member.page.url()).pathname,
        path,
        "bootstrap-target-preserved",
      );
      assert.equal(
        await member.page.getByRole("log", { name: "Nachrichten" }).count(),
        0,
        "unknown-session-hides-private-content",
      );
      assert.ok(blocked >= 1, "bootstrap-request-failed");
      await member.page.screenshot({
        path: `${output}/bootstrap-retry-desktop.png`,
      });
      await member.page.unroute("**/api/auth/session", handler);
      const response = member.page.waitForResponse(
        (row) =>
          row.url().endsWith("/api/auth/session") && row.status() === 200,
      );
      await member.page
        .getByRole("button", { name: "Sitzung erneut prüfen", exact: true })
        .click();
      await response;
      await member.page.getByRole("log", { name: "Nachrichten" }).waitFor();
      assert.equal(
        new URL(member.page.url()).pathname,
        path,
        "retry-restores-same-target",
      );
      assert.equal(
        await member.page
          .getByRole("button", { name: "Sitzung erneut prüfen", exact: true })
          .count(),
        0,
        "bootstrap-warning-cleared",
      );
      await liveTopic(member, channel);
      await liveTopic(member, second);
    },
  );

  await check(
    "failed logout warning survives failed login and Retry revokes cookie",
    async () => {
      await settings(member, { toast: true, desktop: false });
      let blocked = 0;
      const handler = async (route) => {
        blocked++;
        await fail(route);
      };
      await member.page.route("**/api/auth/logout", handler);
      await member.page
        .getByRole("button", { name: "Abmelden", exact: true })
        .click();
      await member.page.waitForURL((url) => url.pathname === "/login");
      await member.page
        .getByRole("button", { name: "Erneut abmelden", exact: true })
        .waitFor();
      assert.equal(blocked, 1, "logout-request-failed");
      const loginFailure = member.page.waitForResponse(
        (row) => row.url().endsWith("/api/auth/login") && row.status() === 401,
      );
      await login(member, member.email, `${password}-wrong`);
      await loginFailure;
      await member.page
        .getByRole("button", { name: "Anmelden", exact: true })
        .waitFor();
      await member.page
        .getByRole("button", { name: "Erneut abmelden", exact: true })
        .waitFor();
      await member.page.screenshot({
        path: `${output}/failed-login-keeps-logout-retry.png`,
      });
      await member.page.unroute("**/api/auth/logout", handler);
      const logoutSuccess = member.page.waitForResponse(
        (row) => row.url().endsWith("/api/auth/logout") && row.status() === 200,
      );
      await member.page
        .getByRole("button", { name: "Erneut abmelden", exact: true })
        .click();
      await logoutSuccess;
      await until(
        () =>
          member.page
            .getByRole("button", { name: "Erneut abmelden", exact: true })
            .count(),
        (count) => count === 0,
        "logout-warning-cleared",
      );
      assert.equal(
        (await api(member, "/auth/session")).body.user,
        null,
        "retry-revokes-server-session",
      );
      await login(member, member.email);
      await member.page.waitForURL((url) => url.pathname !== "/login");
      await member.page.goto(`${base}${path}`);
      await member.page.getByRole("log", { name: "Nachrichten" }).waitFor();
    },
  );

  await check(
    "notification permission status and retry use explicit browser mock",
    async () => {
      await settings(member, { toast: false, desktop: false });
      await member.page
        .getByText(
          "Der Browser benötigt noch deine Erlaubnis für Benachrichtigungen.",
          { exact: true },
        )
        .waitFor();
      await member.page.evaluate(() => {
        window.__notificationSeam.reject = true;
      });
      await member.page
        .getByRole("button", {
          name: "Benachrichtigungen erlauben",
          exact: true,
        })
        .click();
      await member.page
        .getByRole("alert")
        .filter({ hasText: "Die Berechtigung konnte nicht abgefragt werden" })
        .waitFor();
      await member.page.evaluate(() => {
        window.__notificationSeam.reject = false;
      });
      await member.page
        .getByRole("button", {
          name: "Benachrichtigungen erlauben",
          exact: true,
        })
        .click();
      await member.page
        .getByText("Browser-Benachrichtigungen sind erlaubt.", { exact: true })
        .waitFor();
      assert.equal(
        await member.page.evaluate(() => window.__notificationSeam.requested),
        2,
        "mock-permission-retry",
      );
      await member.page.evaluate(() => {
        window.__notificationSeam.permission = "denied";
        window.dispatchEvent(new Event("focus"));
      });
      await member.page
        .getByText("Browser-Benachrichtigungen sind blockiert.", {
          exact: false,
        })
        .waitFor();
      assert.equal(
        await member.page
          .getByRole("button", {
            name: "Benachrichtigungen erlauben",
            exact: true,
          })
          .count(),
        0,
        "denied-status-has-browser-instructions",
      );
      await member.page.evaluate(() => {
        delete window.Notification;
        window.dispatchEvent(new Event("focus"));
      });
      await member.page
        .getByText(
          "Dieser Browser unterstützt hier keine Desktop-Benachrichtigungen.",
          { exact: true },
        )
        .waitFor();
      await member.page.evaluate(() => {
        window.Notification = window.__notificationSeam.MockNotification;
        window.__notificationSeam.permission = "granted";
        window.dispatchEvent(new Event("focus"));
      });
      await member.page
        .getByText("Browser-Benachrichtigungen sind erlaubt.", { exact: true })
        .waitFor();
      await member.page.setViewportSize({ width: 390, height: 844 });
      await member.page.screenshot({
        path: `${output}/notification-settings-mobile.png`,
      });
      await member.page.setViewportSize({ width: 1280, height: 720 });
      await member.page.locator("#desktop-notify").setChecked(true);
      await goBack(member);
      await liveTopic(member, channel);
      await liveTopic(member, second);
    },
  );

  await check(
    "desktop preference works with toasts off in hidden open chat; own and denied excluded",
    async () => {
      await setHidden(member, true);
      const message = await send(owner, channel, `desktop-only-${suffix}`);
      await delivered(member, message);
      await until(
        () => notifications(member),
        (rows) => rows.length === 1,
        "desktop-created",
      );
      const [notice] = await notifications(member);
      assert.equal(notice.options.body, message.content, "desktop-preview");
      assert.equal(notice.options.silent, true, "desktop-silent");
      assert.ok(
        notice.title.startsWith("Recovery owner · #"),
        "desktop-author-channel-label",
      );
      assert.equal(
        await member.page
          .locator('[aria-live="polite"] button')
          .filter({ hasText: message.content })
          .count(),
        0,
        "toast-disabled-independent",
      );
      const own = await send(member, channel, `own-${suffix}`);
      await delivered(member, own);
      assert.equal(
        (await notifications(member)).length,
        1,
        "own-message-excluded",
      );
      await member.page.evaluate(() => {
        window.__notificationSeam.permission = "denied";
      });
      const denied = await send(owner, channel, `denied-${suffix}`);
      await delivered(member, denied);
      assert.equal(
        (await notifications(member)).length,
        1,
        "permission-denied-excluded",
      );
      await member.page.evaluate(() => {
        window.__notificationSeam.permission = "granted";
      });
      await setHidden(member, false);
    },
  );

  await check(
    "toast preference works with desktop off and opens correct channel",
    async () => {
      await settings(member, { toast: true, desktop: false });
      await goBack(member);
      await setHidden(member, true);
      const message = await send(owner, second, `toast-only-${suffix}`);
      await delivered(member, message);
      const toast = member.page
        .locator('[aria-live="polite"] button')
        .filter({ hasText: message.content });
      await toast.waitFor();
      assert.equal(
        (await notifications(member)).length,
        1,
        "desktop-disabled-independent",
      );
      await setHidden(member, false);
      await toast.click();
      await member.page.waitForURL((url) => url.pathname === other);
      await member.page
        .getByRole("log", { name: "Nachrichten" })
        .getByText(message.content, { exact: true })
        .waitFor();
    },
  );

  await check(
    "mock desktop clicks navigate server and DM notifications",
    async () => {
      await settings(member, { toast: false, desktop: true });
      await goBack(member);
      await setHidden(member, true);
      const channelMessage = await send(
        owner,
        channel,
        `desktop-channel-click-${suffix}`,
      );
      await delivered(member, channelMessage);
      await until(
        () => notifications(member),
        (rows) => rows.length === 2,
        "desktop-channel-ready",
      );
      await member.page.evaluate(() =>
        window.__notificationSeam.notices[1].onclick(),
      );
      await member.page.waitForURL((url) => url.pathname === path);
      assert.equal(
        (await notifications(member))[1].closed,
        true,
        "clicked-notification-closed",
      );
      const dmResponse = await api(owner, "/dms", "POST", {
        user_id: member.id,
      });
      assert.ok([200, 201].includes(dmResponse.status), "fixture-dm-create");
      const dm = dmResponse.body.id;
      await liveTopic(member, dm);
      const dmMessage = await send(owner, dm, `desktop-dm-click-${suffix}`);
      await delivered(member, dmMessage);
      await until(
        () => notifications(member),
        (rows) => rows.length === 3,
        "desktop-dm-ready",
      );
      await member.page.evaluate(() =>
        window.__notificationSeam.notices[2].onclick(),
      );
      await member.page.waitForURL((url) => url.pathname === `/d/${dm}`);
      await member.page
        .getByRole("log", { name: "Nachrichten" })
        .getByText(dmMessage.content, { exact: true })
        .waitFor();
      await setHidden(member, false);
    },
  );

  await check(
    "logout clears toast and stale desktop click cannot navigate new account",
    async () => {
      await settings(member, { toast: true, desktop: true });
      await goBack(member);
      await setHidden(member, true);
      const message = await send(owner, second, `account-fenced-${suffix}`);
      await delivered(member, message);
      await until(
        () => notifications(member),
        (rows) => rows.length === 4,
        "account-fence-notification-ready",
      );
      await member.page
        .locator('[aria-live="polite"] button')
        .filter({ hasText: message.content })
        .waitFor();
      await setHidden(member, false);
      await member.page.locator('a[href="/settings"]').first().click();
      await member.page
        .getByRole("button", { name: "Abmelden", exact: true })
        .click();
      await member.page.waitForURL((url) => url.pathname === "/login");
      assert.equal(
        await member.page
          .locator('[aria-live="polite"] button')
          .filter({ hasText: message.content })
          .count(),
        0,
        "logout-clears-account-toast",
      );
      await login(member, replacement.email);
      await member.page.waitForURL((url) => url.pathname !== "/login");
      const before = member.page.url();
      const noticeCountBefore = await member.page.evaluate(
        () => window.__notificationSeam.notices.length,
      );
      await member.page.evaluate(() =>
        window.__notificationSeam.notices[3].onclick(),
      );
      // Observe after the real router transition would have completed, not just synchronously.
      await renderTurn(member);
      assert.equal(
        member.page.url(),
        before,
        "stale-desktop-click-account-fence",
      );
      assert.equal(
        (await api(member, "/auth/session")).body.user.id,
        replacement.id,
        "replacement-account-preserved",
      );
      assert.equal(
        (await notifications(member))[3].closed,
        true,
        "stale-notification-closed",
      );
      assert.equal(noticeCountBefore, 4, "four-notifications-observed");
    },
  );
  await writeFile(
    `${output}/report.json`,
    JSON.stringify(
      {
        checks,
        provenance,
        passed: checks.length,
        browser: {
          engine: "chromium",
          version: browser.version(),
          viewport: "desktop and 390x844",
        },
        seams: [
          "MockNotification constructor/permission/click",
          "document hidden/visibilityState",
        ],
        real: [
          "registration/login/logout/session API",
          "PostgreSQL channel/DM fixtures",
          "WebSocket subscription acknowledgements and delivery",
          "router navigation and rendered preferences",
        ],
        nativeOsPermissionAcceptance: false,
        physicalDeviceAcceptance: false,
      },
      null,
      2,
    ),
  );
} catch (error) {
  console.error(`FAIL ${currentCase}: ${safeError(error)}`);
  await writeFile(
    `${output}/report.json`,
    JSON.stringify(
      {
        checks,
        provenance,
        passed: checks.length,
        failure: safeError(error),
        failedCase: currentCase,
        nativeOsPermissionAcceptance: false,
        physicalDeviceAcceptance: false,
      },
      null,
      2,
    ),
  );
  process.exitCode = 1;
} finally {
  if (server && owner) {
    try {
      await api(owner, `/servers/${server}`, "DELETE");
    } catch {
      /* Isolated local fixture. */
    }
  }
  for (const context of contexts) await context.close();
  await browser.close();
}
