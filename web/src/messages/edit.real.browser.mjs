/* global console, process, setTimeout, clearTimeout */
// Real PATCH/REST races in the browser with WS stopped to isolate HTTP order.
// Uses only the coordinator's loopback API and an owned temporary test server.
import assert from "node:assert/strict";
import { randomBytes, randomUUID } from "node:crypto";
import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath, URL } from "node:url";
import { createServer } from "vite";
import { chromium, firefox } from "playwright";

const apiBase =
  process.env.GELABBER_WEB_TEST_API_URL ?? "http://127.0.0.1:8084";
const apiUrl = new URL(apiBase);
assert(
  ["127.0.0.1", "localhost", "[::1]"].includes(apiUrl.hostname),
  "Only a local test API is allowed",
);
const cache = await mkdtemp(join(tmpdir(), "gelabber-edit-real-"));
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
const engine = process.env.GELABBER_WEB_TEST_BROWSER ?? "chromium";
assert(["chromium", "firefox"].includes(engine));
const browser = await (engine === "firefox" ? firefox : chromium).launch({
  headless: true,
  ...(engine === "chromium" ? { args: ["--no-sandbox"] } : {}),
});
console.log("Browser", engine, browser.version());
const password = randomBytes(24).toString("base64url");
const suffix = randomUUID();
const contexts = [];
const releaseGates = [];
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
async function cacheRows(page, channelId) {
  return page.evaluate(async (id) => {
    const { queryClient } = await import("/src/queryClient.ts");
    const { takeStamp } = await import("/src/auth/scope.ts");
    const { messageKeys } = await import("/src/messages/queries.ts");
    const stamp = takeStamp();
    return queryClient
      .getQueryData(messageKeys.channel(stamp.userId, stamp.generation, id))
      ?.pages.flatMap((p) => p.messages)
      .map((m) => ({ id: m.id, revision: m.revision, content: m.content }));
  }, channelId);
}
async function refreshHistory(page, channelId) {
  await page.evaluate(async (id) => {
    const { queryClient } = await import("/src/queryClient.ts");
    const { takeStamp } = await import("/src/auth/scope.ts");
    const { messageQueryOptions } = await import("/src/messages/queries.ts");
    const stamp = takeStamp();
    await queryClient.fetchInfiniteQuery({
      ...messageQueryOptions(queryClient, stamp.userId, stamp.generation, id),
      staleTime: 0,
    });
  }, channelId);
}
async function deadline(promise, label) {
  let timer;
  try {
    return await Promise.race([
      promise,
      new Promise((_, reject) => {
        timer = setTimeout(
          () => reject(new Error(`Timed out: ${label}`)),
          5000,
        );
      }),
    ]);
  } finally {
    clearTimeout(timer);
  }
}
try {
  owner = await account("owner");
  const created = await apiCall(owner.page, "/servers", "POST", {
    name: `Edit REST Order ${suffix}`,
  });
  serverId = created.id;
  const detail = await apiCall(owner.page, `/servers/${serverId}`);
  const channelId = detail.channels.find((c) => c.kind === "text").id;
  await navigate(owner.page, `/s/${serverId}/c/${channelId}`);
  await owner.page.locator("form textarea").last().waitFor();
  await owner.page.evaluate(async () => {
    const { getGateway } = await import("/src/ws/client.ts");
    getGateway().stop();
  });
  for (const restOrder of ["older", "newer"]) {
    const original = await apiCall(
      owner.page,
      `/channels/${channelId}/messages`,
      "POST",
      {
        content: `${restOrder} original`,
      },
    );
    await refreshHistory(owner.page, channelId);
    const oldSnapshot = await owner.context.request.get(
      `${base}/api/channels/${channelId}/messages?limit=50`,
    );
    assert.equal(oldSnapshot.status(), 200);
    let release, started;
    const gate = new Promise((r) => {
      release = r;
    });
    releaseGates.push(release);
    const heldResponse = new Promise((r) => {
      started = r;
    });
    let captured = false;
    const patchRoute = `**/api/messages/${original.id}`;
    await owner.page.route(patchRoute, async (route) => {
      if (route.request().method() !== "PATCH" || captured) {
        await route.continue();
        return;
      }
      captured = true;
      const actual = await route.fetch();
      assert.equal(actual.status(), 200);
      const canonical = await actual.json();
      started(canonical);
      await gate;
      await route.fulfill({ response: actual });
    });
    await owner.page.evaluate(
      async ({ id, channelId, content }) => {
        const { queryClient } = await import("/src/queryClient.ts");
        const { editMessageOptions } = await import("/src/messages/queries.ts");
        globalThis.__editProbeDone = queryClient
          .getMutationCache()
          .build(queryClient, editMessageOptions(queryClient, channelId))
          .execute({ id, content });
      },
      { id: original.id, channelId, content: `${restOrder} PATCH` },
    );
    const canonical = await deadline(heldResponse, "committed PATCH response");
    assert(canonical.revision > original.revision);
    assert.equal(
      (await cacheRows(owner.page, channelId)).find((m) => m.id === original.id)
        .content,
      `${restOrder} PATCH`,
    );
    const historyRoute = `**/api/channels/${channelId}/messages*`;
    let expected;
    if (restOrder === "older") {
      await owner.page.route(historyRoute, (route) =>
        route.fulfill({ response: oldSnapshot }),
      );
      await refreshHistory(owner.page, channelId);
      assert.equal(
        (await cacheRows(owner.page, channelId)).find(
          (m) => m.id === original.id,
        ).content,
        original.content,
      );
      await owner.page.unroute(historyRoute);
      expected = canonical;
    } else {
      expected = await apiCall(
        owner.page,
        `/messages/${original.id}`,
        "PATCH",
        {
          content: "newer REST after committed PATCH",
        },
      );
      assert(expected.revision > canonical.revision);
      await refreshHistory(owner.page, channelId);
      assert.equal(
        (await cacheRows(owner.page, channelId)).find(
          (m) => m.id === original.id,
        ).revision,
        expected.revision,
      );
    }
    release();
    await deadline(
      owner.page.evaluate(async () => {
        await globalThis.__editProbeDone;
        delete globalThis.__editProbeDone;
      }),
      "PATCH settlement",
    );
    const row = (await cacheRows(owner.page, channelId)).find(
      (m) => m.id === original.id,
    );
    assert.equal(row.content, expected.content);
    assert.equal(row.revision, expected.revision);
    await owner.page
      .getByRole("log")
      .getByText(expected.content, { exact: false })
      .waitFor();
    await owner.page.unroute(patchRoute);
    console.log(
      `PASS real committed held PATCH with ${restOrder} REST snapshot: correct DB revision and visible text, no WS repair`,
    );
  }
} finally {
  for (const release of releaseGates) release();
  try {
    if (owner && serverId) {
      await apiCall(owner.page, `/servers/${serverId}`, "DELETE");
      console.log("PASS owned test-server cleanup 204");
    }
  } finally {
    for (const context of contexts) await context.close();
    await browser.close();
    await server.close();
    await rm(cache, { recursive: true, force: true });
  }
}
