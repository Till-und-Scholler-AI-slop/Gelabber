/* global location, console, process, document, Event */
// Runs the actual React router and auth bootstrap against a local real API.
// Start the coordinator's API at 8080, then: node src/auth/session.real.browser.mjs
import assert from "node:assert/strict";
import { randomBytes, randomUUID } from "node:crypto";
import { mkdtemp, rm, readFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath, URL } from "node:url";
import { createServer } from "vite";
import { chromium } from "playwright";

const apiBase =
  process.env.GELABBER_WEB_TEST_API_URL ?? "http://127.0.0.1:8084";
const apiUrl = new URL(apiBase);
assert(
  ["127.0.0.1", "localhost", "[::1]"].includes(apiUrl.hostname),
  "Only a local test API is allowed",
);
const cache = await mkdtemp(join(tmpdir(), "gelabber-send-real-"));
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
async function apiCall(page, path, method = "GET", body) {
  return page.evaluate(
    async ({ path, method, body }) => {
      const { api } = await import("/src/api/client.ts");
      return api(path, { method, body });
    },
    { path, method, body },
  );
}
async function navigate(page, path) {
  await page.evaluate(async (path) => {
    const { router } = await import("/src/routes.tsx");
    await router.navigate({ to: path });
  }, path);
}
async function send(page, text) {
  await page.locator("form textarea").last().fill(text);
  await page.getByRole("button", { name: "Senden", exact: true }).click();
}
try {
  owner = await account("owner");
  const created = await apiCall(owner.page, "/servers", "POST", {
    name: `Send Recovery ${suffix}`,
  });
  serverId = created.id;
  const detail = await apiCall(owner.page, `/servers/${serverId}`);
  const channelId = detail.channels.find((c) => c.kind === "text").id;
  const path = `/s/${serverId}/c/${channelId}`;
  const guest = await account("guest");
  const invite = await apiCall(
    owner.page,
    `/servers/${serverId}/invites`,
    "POST",
    {},
  );
  await apiCall(guest.page, `/invites/${invite.code}/join`, "POST");
  await navigate(owner.page, path);
  await navigate(guest.page, path);
  await owner.page.locator("form textarea").last().waitFor();
  const messageRoute = `**/api/channels/${channelId}/messages*`;

  let release, started;
  const gate = new Promise((r) => {
    release = r;
  });
  const pending = new Promise((r) => {
    started = r;
  });
  let posts = 0;
  await owner.page.route(messageRoute, async (route) => {
    if (route.request().method() === "POST") {
      posts++;
      if (route.request().postDataJSON().content === "late A") {
        started();
        await gate;
        await route.fulfill({
          status: 503,
          contentType: "application/json",
          body: JSON.stringify({ error: "unavailable" }),
        });
        return;
      }
    }
    await route.continue();
  });
  await send(owner.page, "late A");
  await pending;
  await send(owner.page, "parallel B");
  await guest.page
    .getByRole("log")
    .getByText("parallel B", { exact: true })
    .waitFor();
  await navigate(owner.page, "/d");
  release();
  await navigate(owner.page, path);
  await owner.page
    .getByRole("button", { name: "Sendung wiederholen", exact: true })
    .waitFor();
  assert.equal(posts, 2);
  await owner.page.unroute(messageRoute);
  await owner.page
    .getByRole("button", { name: "Sendung wiederholen", exact: true })
    .click();
  await guest.page
    .getByRole("log")
    .getByText("late A", { exact: true })
    .waitFor();
  const list = await apiCall(owner.page, `/channels/${channelId}/messages`);
  assert.equal(list.messages.filter((m) => m.content === "late A").length, 1);
  assert.equal(
    list.messages.filter((m) => m.content === "parallel B").length,
    1,
  );
  console.log(
    "PASS real app parallel late failure → navigation → exact retry, no duplicate",
  );

  let gets = 0,
    failHistory = true;
  await owner.page.route(messageRoute, async (route) => {
    if (route.request().method() === "GET" && ++gets && failHistory) {
      await route.fulfill({
        status: 503,
        contentType: "application/json",
        body: JSON.stringify({ error: "unavailable" }),
      });
      return;
    }
    await route.continue();
  });
  await owner.page.reload();
  await owner.page
    .getByRole("button", { name: "Erneut laden", exact: true })
    .waitFor();
  // No periodic/browser-focus retry; three seconds exceeds the former retry delay.
  await owner.page.waitForTimeout(3000);
  const failedGets = gets;
  assert(failedGets > 0 && failedGets <= 2);
  failHistory = false;
  await owner.page
    .getByRole("button", { name: "Erneut laden", exact: true })
    .click();
  await owner.page
    .getByRole("log")
    .getByText("parallel B", { exact: true })
    .waitFor();
  assert.equal(gets, failedGets + 1);
  await owner.page.unroute(messageRoute);
  console.log(
    "PASS real app initial 503 → visible explicit retry with bounded requests",
  );

  // Seed >50 with separate per-account budgets; real REST provides both pages.
  for (let i = 0; i < 51; i++)
    await apiCall(
      i % 2 ? guest.page : owner.page,
      `/channels/${channelId}/messages`,
      "POST",
      { content: `paging-${i}` },
    );
  await owner.page.reload();
  await owner.page
    .getByRole("log")
    .getByText("paging-50", { exact: true })
    .waitFor();
  let paging = 0;
  await owner.page.route(messageRoute, async (route) => {
    if (
      new URL(route.request().url()).searchParams.has("before") &&
      ++paging === 1
    ) {
      await route.fulfill({
        status: 503,
        contentType: "application/json",
        body: JSON.stringify({ error: "unavailable" }),
      });
      return;
    }
    await route.continue();
  });
  await owner.page.getByRole("log").evaluate((el) => {
    el.scrollTop = 0;
    el.dispatchEvent(new Event("scroll"));
  });
  await owner.page
    .getByRole("button", { name: "Erneut laden", exact: true })
    .waitFor();
  await owner.page.waitForTimeout(3000);
  assert.equal(paging, 1);
  await owner.page
    .getByRole("button", { name: "Erneut laden", exact: true })
    .click();
  await owner.page
    .getByRole("log")
    .getByText("paging-0", { exact: true })
    .waitFor();
  assert.equal(paging, 2);
  await owner.page.unroute(messageRoute);
  console.log(
    "PASS real app >50 history paging 503 → explicit retry, no paging loop",
  );

  const guestId = (await state(guest.page)).userId;
  const dm = await apiCall(owner.page, "/dms", "POST", { user_id: guestId });
  for (const [kind, , chatPath] of [
    ["channel", channelId, path],
    ["dm", dm.id, `/d/${dm.id}`],
  ]) {
    await navigate(owner.page, chatPath);
    await navigate(guest.page, chatPath);
    for (const extension of ["jpg", "png", "webp"]) {
      const fixture = fileURLToPath(
        new URL(`../../scripts/fixtures/smoke.${extension}`, import.meta.url),
      );
      const expected = await readFile(fixture);
      await owner.page.locator('input[type="file"]').setInputFiles(fixture);
      assert(
        await owner.page
          .locator("form textarea")
          .last()
          .evaluate((el) => el === document.activeElement),
        "PR112 focus retained",
      );
      await send(owner.page, `${kind}-${extension}`);
      await guest.page
        .getByRole("log")
        .getByText(`${kind}-${extension}`, { exact: true })
        .waitFor();
      await guest.page.reload();
      const image = guest.page
        .getByRole("log")
        .getByRole("img", { name: `smoke.${extension}`, exact: true });
      await image.waitFor();
      const rendered = await image.evaluate(async (el) => {
        if (!el.complete)
          await new Promise((r) =>
            el.addEventListener("load", r, { once: true }),
          );
        return el.naturalWidth > 0;
      });
      assert(rendered);
      const bytes = await guest.context.request.get(
        await image.getAttribute("src").then((src) => new URL(src, base).href),
      );
      assert.equal(bytes.status(), 200);
      assert.deepEqual(await bytes.body(), expected);
    }
  }
  console.log(
    "PASS real API/MinIO JPEG PNG WebP channel+DM, second account, reload, byte equality (local HTTP)",
  );
} finally {
  try {
    if (owner && serverId) {
      const status = await owner.page.evaluate(async (id) => {
        const { api } = await import("/src/api/client.ts");
        await api(`/servers/${id}`, { method: "DELETE" });
        return 204;
      }, serverId);
      assert.equal(status, 204);
      console.log("PASS owned test-server cleanup 204");
    }
  } finally {
    for (const context of contexts) await context.close();
    await browser.close();
    await server.close();
    await rm(cache, { recursive: true, force: true });
  }
}
