/* global window, location, console, process, fetch */
// Runs the actual React router and auth bootstrap against a local real API.
// Start the coordinator's API at 8080, then: node src/auth/session.real.browser.mjs
import assert from "node:assert/strict";
import { randomBytes, randomUUID } from "node:crypto";
import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath, URL } from "node:url";
import { createServer } from "vite";
import { chromium } from "playwright";

const apiBase =
  process.env.GELABBER_AUTH_TEST_API_URL ?? "http://127.0.0.1:8080";
const apiUrl = new URL(apiBase);
assert(
  ["127.0.0.1", "localhost", "[::1]"].includes(apiUrl.hostname),
  "Only a local test API is allowed",
);
const cache = await mkdtemp(join(tmpdir(), "gelabber-auth-real-"));
const server = await createServer({
  root: fileURLToPath(new URL("../../", import.meta.url)),
  configLoader: "runner",
  cacheDir: cache,
  server: {
    host: "127.0.0.1",
    port: 0,
    proxy: {
      "/api": { target: apiBase, changeOrigin: false },
      "/ws": { target: apiBase, changeOrigin: false, ws: true },
    },
  },
  logLevel: "error",
});
await server.listen();
const base = `http://127.0.0.1:${server.httpServer.address().port}`;
const browser = await chromium.launch({
  headless: true,
  args: ["--no-sandbox"],
});
const password = randomBytes(24).toString("base64url");
const suffix = randomUUID();
const contexts = [];
let owner, serverId;
async function account(name) {
  const context = await browser.newContext();
  contexts.push(context);
  const page = await context.newPage();
  page.setDefaultTimeout(5000);
  await page.goto(`${base}/register`);
  await page.getByLabel("Name", { exact: true }).fill(`Auth Test ${name}`);
  await page
    .getByLabel("E-Mail-Adresse")
    .fill(`auth-${suffix}-${name}@example.test`);
  await page.getByLabel("Passwort", { exact: true }).fill(password);
  await page.getByRole("button", { name: "Registrieren", exact: true }).click();
  await page.waitForURL((url) => url.pathname !== "/register");
  return { page, context };
}
async function state(page) {
  return page.evaluate(async () => {
    const { useSession } = await import("/src/auth/session.ts");
    const { queryScopeUser } = await import("/src/queryClient.ts");
    const { takeStamp } = await import("/src/auth/scope.ts");
    return {
      status: useSession.getState().status,
      userId: useSession.getState().user?.id,
      queryUserId: queryScopeUser(),
      stamp: takeStamp(),
      path: location.pathname,
    };
  });
}
try {
  owner = await account("owner");
  await owner.page
    .getByRole("button", { name: "Server erstellen", exact: true })
    .first()
    .click();
  let dialog = owner.page.getByRole("dialog", { name: "Server erstellen" });
  await dialog
    .getByLabel("Name", { exact: true })
    .fill(`Auth Invite ${suffix}`);
  await dialog.getByRole("button", { name: "Erstellen", exact: true }).click();
  await owner.page.waitForURL(/\/s\/[^/]+\/c\//);
  serverId = new URL(owner.page.url()).pathname.split("/")[2];
  await owner.page
    .getByRole("button", { name: "Leute einladen", exact: true })
    .click();
  dialog = owner.page.getByRole("dialog", { name: /Einladen zu/ });
  await dialog
    .getByRole("button", { name: "Link erstellen", exact: true })
    .click();
  const invite = await dialog
    .getByRole("textbox", { name: "Einladungslink" })
    .inputValue();
  await dialog.getByRole("button", { name: "Fertig", exact: true }).click();
  const guest = await account("guest");
  const before = await state(guest.page);
  await guest.page.goto(invite);
  try {
    await guest.page
      .getByRole("button", { name: "Beitreten", exact: true })
      .waitFor();
  } catch (error) {
    console.log("Invite bootstrap diagnosis", {
      ...(await state(guest.page)),
      body: await guest.page.locator("main").innerText(),
    });
    throw error;
  }
  const after = await state(guest.page);
  assert.equal(after.status, "authenticated");
  assert.equal(after.userId, before.userId);
  assert.equal(after.queryUserId, before.userId);
  await guest.page
    .getByRole("button", { name: "Beitreten", exact: true })
    .click();
  await guest.page.waitForURL(/\/s\//);
  console.log(
    "PASS real API register → document navigation → invite preview and join",
  );

  // Hold the authoritative real response until the initial pageshow/focus
  // events have run. The router must await this same bootstrap, not redirect.
  let release, started;
  const gate = new Promise((resolve) => {
    release = resolve;
  });
  const responseStarted = new Promise((resolve) => {
    started = resolve;
  });
  let held = false;
  await guest.page.route("**/api/auth/session", async (route) => {
    const response = await route.fetch();
    if (!held) {
      held = true;
      started();
      await gate;
    }
    await route.fulfill({ response });
  });
  await guest.context.addInitScript(() => {
    window.initialPageshow = false;
    window.addEventListener("pageshow", () => {
      window.initialPageshow = true;
    });
  });
  await guest.page.goto(invite, { waitUntil: "domcontentloaded" });
  await responseStarted;
  await guest.page.waitForFunction(() => window.initialPageshow);
  const pending = await state(guest.page);
  release();
  assert.equal(
    pending.status,
    "unknown",
    "An uninitialized tab must keep its bootstrap pending",
  );
  await guest.page
    .getByRole("link", { name: "Zum Server", exact: true })
    .waitFor();
  const settled = await state(guest.page);
  assert.equal(settled.userId, before.userId);
  assert.equal(settled.queryUserId, before.userId);
  console.log(
    "PASS real API held bootstrap during pageshow → authenticated invite member view",
  );
  await guest.page.unroute("**/api/auth/session");
  const other = await guest.context.newPage();
  other.setDefaultTimeout(5000);
  await other.goto(`${base}/profile`);
  await other.getByLabel("Name", { exact: true }).waitFor();
  let releaseReconcile, reconcileStarted;
  const reconcileGate = new Promise((resolve) => {
    releaseReconcile = resolve;
  });
  const reconciliationStarted = new Promise((resolve) => {
    reconcileStarted = resolve;
  });
  let first = true;
  await guest.page.route("**/api/auth/session", async (route) => {
    const response = await route.fetch();
    if (first) {
      first = false;
      reconcileStarted();
      await reconcileGate;
    }
    await route.fulfill({ response });
  });
  await other.evaluate(
    async ({ email, password }) => {
      const { login } = await import("/src/auth/session.ts");
      await login(email, password);
    },
    { email: `auth-${suffix}-owner@example.test`, password },
  );
  await reconciliationStarted;
  assert.equal((await state(guest.page)).status, "unknown");
  await guest.page.evaluate(() => {
    window.guardDone = false;
    window.guardNavigation = import("/src/routes.tsx")
      .then(({ router }) => router.navigate({ to: "/profile" }))
      .then(() => {
        window.guardDone = true;
      });
  });
  // Await the exact shared bootstrap directly as well as through the real guard.
  await guest.page.evaluate(async () => {
    const { ensureSession } = await import("/src/auth/session.ts");
    window.bootstrapDone = false;
    window.bootstrapWaiting = ensureSession().then(() => {
      window.bootstrapDone = true;
    });
  });
  const loading = await guest.page.evaluate(() => ({
    guard: window.guardDone,
    bootstrap: window.bootstrapDone,
  }));
  releaseReconcile();
  assert.deepEqual(loading, { guard: false, bootstrap: false });
  await guest.page.evaluate(() =>
    Promise.all([window.guardNavigation, window.bootstrapWaiting]),
  );
  await guest.page.waitForURL((url) => url.pathname === "/profile");
  assert.equal(
    await guest.page.getByLabel("Name", { exact: true }).inputValue(),
    "Auth Test owner",
  );
  const ownerState = await state(owner.page),
    reconciled = await state(guest.page);
  assert.equal(reconciled.userId, ownerState.userId);
  assert.equal(reconciled.queryUserId, ownerState.userId);
  console.log(
    "PASS real API cross-tab change → held reconciliation → route guard awaits current identity",
  );
  await other.evaluate(async () => {
    const { logout } = await import("/src/auth/session.ts");
    await logout();
  });
  await guest.page.waitForURL((url) => url.pathname === "/login");
  assert.equal((await state(guest.page)).status, "anonymous");
  await guest.page
    .getByRole("button", { name: "Anmelden", exact: true })
    .waitFor();
  console.log(
    "PASS real API confirmed cross-tab logout → anonymous route redirect",
  );
  await guest.page.unroute("**/api/auth/session");
  await guest.page.goto(invite);
  await guest.page
    .getByRole("button", { name: "Anmelden", exact: true })
    .waitFor();
  await guest.page
    .getByLabel("E-Mail-Adresse")
    .fill(`auth-${suffix}-guest@example.test`);
  await guest.page.getByLabel("Passwort", { exact: true }).fill(password);
  await guest.page
    .getByRole("button", { name: "Anmelden", exact: true })
    .click();
  await guest.page.waitForURL(
    (url) => url.pathname === new URL(invite).pathname,
  );
  await guest.page
    .getByRole("link", { name: "Zum Server", exact: true })
    .waitFor();
  assert.equal((await state(guest.page)).userId, before.userId);
  console.log(
    "PASS real API anonymous invite → login → original invite target",
  );
  await guest.page.reload();
  await guest.page
    .getByRole("link", { name: "Zum Server", exact: true })
    .waitFor();
  assert.equal((await state(guest.page)).userId, before.userId);
  console.log(
    "PASS real API hard reload after login → invite without false anonymous redirect",
  );
  console.log(`Browser: Chromium ${browser.version()}`);
} finally {
  try {
    if (owner && serverId) {
      const status = await owner.page.evaluate(async (id) => {
        const session = await fetch("/api/auth/session").then((response) =>
          response.json(),
        );
        return (
          await fetch(`/api/servers/${id}`, {
            method: "DELETE",
            headers: { "X-CSRF-Token": session.csrf_token },
          })
        ).status;
      }, serverId);
      console.log(`Test server cleanup: ${status}`);
      assert.equal(status, 204);
    }
  } finally {
    for (const context of contexts) await context.close();
    await browser.close();
    await server.close();
    await rm(cache, { recursive: true, force: true });
  }
}
