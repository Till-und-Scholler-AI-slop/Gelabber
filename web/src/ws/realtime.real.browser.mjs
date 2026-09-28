/* global location, console, process, setTimeout, clearTimeout */
// Actual React/Query/Gateway tests against the coordinator's local05a API.
// Defaults: API8084, Redis56379. Resets only the created UUID channel's keys.
import assert from "node:assert/strict";
import { execFile } from "node:child_process";
import { promisify } from "node:util";
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
const cache = await mkdtemp(join(tmpdir(), "gelabber-realtime-real-"));
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
const cases = new Set(
  (process.env.GELABBER_WEB_TEST_CASES ?? "revisions,epoch,dm").split(","),
);
assert([...cases].every((name) => ["revisions", "epoch", "dm"].includes(name)));
let owner, serverId;
async function account(name) {
  const context = await browser.newContext();
  contexts.push(context);
  const page = await context.newPage();
  const discovered = new Set();
  page.on("websocket", (socket) =>
    socket.on("framereceived", ({ payload }) => {
      const frame = JSON.parse(String(payload));
      if (frame.op === "dm" && typeof frame.c === "string")
        discovered.add(frame.c);
    }),
  );
  page.setDefaultTimeout(5000);
  await page.goto(`${base}/register`);
  await page.getByLabel("Name", { exact: true }).fill(`Auth Test ${name}`);
  await page
    .getByLabel("E-Mail-Adresse")
    .fill(`auth-${suffix}-${name}@example.test`);
  await page.getByLabel("Passwort", { exact: true }).fill(password);
  await page.getByRole("button", { name: "Registrieren", exact: true }).click();
  await page.waitForURL((url) => url.pathname !== "/register");
  return { page, context, discovered };
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
const redisPort = process.env.GELABBER_WEB_TEST_REDIS_PORT ?? "56379";
const redisCli =
  process.env.GELABBER_WEB_TEST_REDIS_CLI ??
  "/tmp/gelabber-toolchain/redis-8.10.1/src/redis-cli";
const run = promisify(execFile);
async function resetTopic(channelId) {
  assert(
    /^[0-9a-f-]{36}$/.test(channelId),
    "Only the test-created UUID topic can be reset",
  );
  const seq = `gb:n:c:${channelId}`;
  await run(redisCli, [
    "-h",
    "127.0.0.1",
    "-p",
    redisPort,
    "DEL",
    seq,
    `gb:l:c:${channelId}`,
    `${seq}:ep`,
    `${seq}:delivery`,
  ]);
}
async function cursor(page, channelId) {
  return page.evaluate(async (id) => {
    const { getGateway } = await import("/src/ws/client.ts");
    const gateway = getGateway();
    return (
      gateway.topicCursorsSnapshot?.get(`c:${id}`) ??
      (gateway.cursorsSnapshot.has(`c:${id}`)
        ? { n: gateway.cursorsSnapshot.get(`c:${id}`) }
        : undefined)
    );
  }, channelId);
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
async function until(check, label) {
  const deadline = Date.now() + 5000;
  while (Date.now() < deadline) {
    if (await check()) return;
    await new Promise((r) => setTimeout(r, 20));
  }
  console.log(
    "recovery diagnosis",
    await contexts[1]?.pages()[0]?.evaluate(async () => {
      const { getGateway } = await import("/src/ws/client.ts");
      const { queryClient } = await import("/src/queryClient.ts");
      return {
        cursors: [
          ...(getGateway().topicCursorsSnapshot ??
            getGateway().cursorsSnapshot),
        ].map(([, v]) => v),
        gaps: getGateway()
          .gapRecoveries()
          .map((g) => g.version),
        queries: queryClient
          .getQueryCache()
          .getAll()
          .map((q) => ({
            kind: q.queryKey[3],
            state: q.state.status,
            fetching: q.state.fetchStatus,
            error: q.state.error?.code,
            status: q.state.error?.status,
          })),
      };
    }),
  );
  throw new Error(`Timed out: ${label}`);
}
try {
  owner = await account("owner");
  const created = await apiCall(owner.page, "/servers", "POST", {
    name: `Realtime Recovery ${suffix}`,
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
  const one = await apiCall(
    owner.page,
    `/channels/${channelId}/messages`,
    "POST",
    { content: "initial revision" },
  );
  const two = await apiCall(
    owner.page,
    `/channels/${channelId}/messages`,
    "POST",
    { content: "will be deleted" },
  );
  assert(Number.isInteger(one.revision));
  await guest.page
    .getByRole("log")
    .getByText("will be deleted", { exact: true })
    .waitFor();
  const stale = await guest.context.request.get(
    `${base}/api/channels/${channelId}/messages?limit=50`,
  );
  assert.equal(stale.status(), 200);
  const edited = await apiCall(owner.page, `/messages/${one.id}`, "PATCH", {
    content: "durable newest revision",
  });
  assert(edited.revision > one.revision);
  await apiCall(owner.page, `/messages/${two.id}`, "DELETE");
  await guest.page
    .getByRole("log")
    .getByText("durable newest revision", { exact: false })
    .waitFor();
  await guest.page
    .getByRole("log")
    .getByText("will be deleted", { exact: true })
    .waitFor({ state: "hidden" });
  const historyRoute = `**/api/channels/${channelId}/messages*`;
  if (cases.has("revisions")) {
    // Real HTTP snapshot captured before edit/delete, delivered by a later read:
    // the per-read 06a journal has already ended, so persistent floors are required.
    await guest.page.route(historyRoute, (route) =>
      route.fulfill({ response: stale }),
    );
    await refreshHistory(guest.page, channelId);
    const protectedRows = await cacheRows(guest.page, channelId);
    assert(
      !protectedRows.some((m) => m.id === two.id),
      "Stale HTTP must not resurrect delete",
    );
    assert.equal(
      protectedRows.find((m) => m.id === one.id)?.revision,
      edited.revision,
    );
    await guest.page.unroute(historyRoute);
    console.log(
      "PASS real DB revisions: later stale HTTP cannot undo edit/delete floors",
    );
  }

  if (cases.has("epoch")) {
    const old = await cursor(guest.page, channelId);
    assert(old.n >= 4);
    await resetTopic(channelId);
    const next = await apiCall(
      owner.page,
      `/channels/${channelId}/messages`,
      "POST",
      { content: "smaller new epoch" },
    );
    await guest.page
      .getByRole("log")
      .getByText("smaller new epoch", { exact: true })
      .waitFor();
    await until(async () => {
      const now = await cursor(guest.page, channelId);
      return now?.ep && now.ep !== old.ep && now.n === 1;
    }, "new epoch revision reconciliation");
    const newRows = await cacheRows(guest.page, channelId);
    assert.equal(newRows.filter((m) => m.id === next.id).length, 1);
    assert(!newRows.some((m) => m.id === two.id));
    console.log(
      "PASS real UUID-scoped Redis reset: smaller new-epoch event accepted and REST reconciled",
    );

    // Exercise a real server gap + authoritative head zero while REST is held.
    const beforeZero = await cursor(guest.page, channelId);
    await resetTopic(channelId);
    let release, started;
    const gate = new Promise((r) => {
      release = r;
    });
    const held = new Promise((r) => {
      started = r;
    });
    await guest.page.route(historyRoute, async (route) => {
      const response = await route.fetch();
      started();
      await gate;
      await route.fulfill({ response });
    });
    await guest.page.evaluate(
      async ({ serverId, channelId, ep }) => {
        const { getGateway } = await import("/src/ws/client.ts");
        getGateway().send({
          op: "s",
          s: serverId,
          c: channelId,
          ep,
          n: 999999,
        });
      },
      { serverId, channelId, ep: beforeZero.ep },
    );
    releaseGates.push(release);
    await deadline(held, "held gap history");
    assert.deepEqual(
      await cursor(guest.page, channelId),
      beforeZero,
      "Pending gap head must not bypass held REST",
    );
    release();
    await until(async () => {
      const now = await cursor(guest.page, channelId);
      return now?.ep && now.ep !== beforeZero.ep && now.n === 0;
    }, "authoritative head zero after REST");
    await guest.page.unroute(historyRoute);
    console.log(
      "PASS real backend gap/ok head zero waits for held browser REST response",
    );
  }

  if (cases.has("dm")) {
    const receiver = await account("receiver");
    await navigate(receiver.page, "/d");
    const receiverId = (await state(receiver.page)).userId;
    await receiver.page.waitForFunction(async () => {
      const { queryClient } = await import("/src/queryClient.ts");
      const { takeStamp } = await import("/src/auth/scope.ts");
      const stamp = takeStamp();
      return Array.isArray(
        queryClient.getQueryData([
          "user",
          stamp.userId,
          stamp.generation,
          "dms",
          "list",
        ]),
      );
    });
    let listStarted, releaseList;
    const listing = new Promise((r) => {
      listStarted = r;
    });
    const listGate = new Promise((r) => {
      releaseList = r;
    });
    await receiver.page.route("**/api/dms", async (route) => {
      const response = await route.fetch();
      listStarted();
      await listGate;
      await route.fulfill({ response });
    });
    const dm = await apiCall(owner.page, "/dms", "POST", {
      user_id: receiverId,
    });
    const first = await apiCall(
      owner.page,
      `/channels/${dm.id}/messages`,
      "POST",
      { content: "first before subscribe" },
    );
    releaseGates.push(releaseList);
    try {
      await deadline(listing, "private DM discovery list");
    } catch (error) {
      releaseList();
      await receiver.page.unroute("**/api/dms");
      await receiver.page.reload();
      await receiver.page.locator(`a[href="/d/${dm.id}"]`).click();
      await receiver.page
        .getByRole("log")
        .getByText("first before subscribe", { exact: true })
        .waitFor();
      assert(receiver.discovered.has(dm.id));
      console.log(
        "CONTROL reload finds actual DM and first message after failed automatic discovery",
      );
      throw error;
    }
    releaseList();
    await receiver.page.locator(`a[href="/d/${dm.id}"]`).waitFor();
    await until(
      async () =>
        (await cacheRows(receiver.page, dm.id))?.some((m) => m.id === first.id),
      "discovery first-message catch-up",
    );
    assert(receiver.discovered.has(dm.id));
    await receiver.page.locator(`a[href="/d/${dm.id}"]`).click();
    await receiver.page
      .getByRole("log")
      .getByText("first before subscribe", { exact: true })
      .waitFor();
    const outsiders = await apiCall(guest.page, "/dms");
    assert(!outsiders.some((d) => d.id === dm.id));
    assert.equal(await cursor(guest.page, dm.id), undefined);
    console.log(
      "PASS real private DM discovery on already-connected recipient: list, topic, first message without reload; outsider excluded",
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
