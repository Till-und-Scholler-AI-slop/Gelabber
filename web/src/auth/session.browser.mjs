/* global window, navigator, console, setTimeout */
// Run with the pinned toolchain: node src/auth/session.browser.mjs
// Real HttpOnly Set-Cookie handling, shared-origin tabs, and held HTTP responses.
import assert from "node:assert/strict";
import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath, URL } from "node:url";
import { createServer } from "vite";
import { chromium } from "playwright";
const users = Object.fromEntries(
  ["A", "B"].map((id) => [
    id,
    {
      id: `user-${id}`,
      email: `${id.toLowerCase()}@example.com`,
      name: id,
      avatar_url: null,
      created_at: "2026-09-28T00:00:00Z",
    },
  ]),
);
const sessions = new Map();
const held = new Map();
const seen = [];
let mode = "";
let serial = 0;
const cookie = (name, value, maxAge = 3600) =>
  `${name}=${value}; Path=/; Max-Age=${maxAge}; HttpOnly; SameSite=Lax`;
function answer(response, status, body, cookies) {
  response.writeHead(status, {
    "Content-Type": "application/json",
    ...(cookies ? { "Set-Cookie": cookies } : {}),
  });
  response.end(JSON.stringify(body));
}
const cache = await mkdtemp(join(tmpdir(), "gelabber-auth-browser-"));
const server = await createServer({
  root: fileURLToPath(new URL("../../", import.meta.url)),
  configFile: false,
  cacheDir: cache,
  server: { host: "127.0.0.1", port: 0 },
  logLevel: "error",
  plugins: [
    {
      name: "auth-cookie-regression",
      configureServer(vite) {
        vite.middlewares.use(async (req, res, next) => {
          if (req.url === "/__auth_test") {
            res.setHeader("Content-Type", "text/html");
            res.end(`<script type="module">
          import * as session from '/src/auth/session.ts';
          import {api} from '/src/api/client.ts';
          window.review = {...session,api};
        </script>`);
            return;
          }
          if (!req.url?.startsWith("/api/")) return next();
          const path = req.url;
          const cookies = Object.fromEntries(
            (req.headers.cookie ?? "")
              .split(";")
              .filter(Boolean)
              .map((pair) => pair.trim().split("=")),
          );
          const user = sessions.get(cookies.gelabber_session) ?? null;
          seen.push({ method: req.method, path, user: user?.id ?? null });
          if (
            req.method !== "GET" &&
            req.headers["x-csrf-token"] !== cookies.gelabber_csrf
          ) {
            answer(res, 403, { error: "csrf_invalid" });
            return;
          }
          if (path === "/api/auth/session") {
            const csrf = cookies.gelabber_csrf ?? `csrf-anon-${++serial}`;
            answer(
              res,
              200,
              { user, csrf_token: csrf },
              cookies.gelabber_csrf
                ? undefined
                : [cookie("gelabber_csrf", csrf)],
            );
            return;
          }
          if (path === "/api/auth/login" || path === "/api/auth/register") {
            let body = "";
            for await (const chunk of req) body += chunk;
            const id = JSON.parse(body).email.startsWith("a") ? "A" : "B";
            const finish = () => {
              if (cookies.gelabber_session)
                sessions.delete(cookies.gelabber_session);
              const token = `session-${++serial}`;
              const csrf = `csrf-${serial}`;
              sessions.set(token, users[id]);
              answer(res, 200, { user: users[id], csrf_token: csrf }, [
                cookie("gelabber_session", token),
                cookie("gelabber_csrf", csrf),
              ]);
            };
            if (mode === "hold-login" && id === "A") held.set("login", finish);
            else finish();
            return;
          }
          if (path === "/api/auth/logout") {
            if (cookies.gelabber_session)
              sessions.delete(cookies.gelabber_session);
            const finish = () =>
              answer(res, 200, { csrf_token: "csrf-out" }, [
                cookie("gelabber_session", "", 0),
                cookie("gelabber_csrf", "csrf-out"),
              ]);
            if (mode === "hold-logout") held.set("logout", finish);
            else finish();
            return;
          }
          if (path === "/api/servers") {
            answer(
              res,
              user ? 200 : 401,
              user ? { owner: user.id } : { error: "unauthenticated" },
            );
            return;
          }
          answer(res, 404, { error: "not_found" });
        });
      },
    },
  ],
});
await server.listen();
const address = server.httpServer.address();
assert(address && typeof address === "object");
const origin = `http://127.0.0.1:${address.port}`;
const browser = await chromium.launch({
  headless: true,
  args: ["--no-sandbox"],
});
async function load(page) {
  await page.goto(`${origin}/__auth_test`);
  await page.waitForFunction(() => !!window.review);
  await page.evaluate(() => window.review.ensureSession());
}
async function waitForHeld(key) {
  const end = Date.now() + 3000;
  while (!held.has(key)) {
    assert(Date.now() < end, `Missing held ${key}`);
    await new Promise((resolve) => setTimeout(resolve, 5));
  }
}
async function assertQueued(page) {
  await page.waitForFunction(async () =>
    (await navigator.locks.query()).pending?.some(
      (lock) => lock.name === "gelabber:auth-cookies",
    ),
  );
}
try {
  {
    mode = "hold-logout";
    held.clear();
    seen.length = 0;
    const context = await browser.newContext();
    const first = await context.newPage();
    const second = await context.newPage();
    await load(first);
    await load(second);
    await first.evaluate(() =>
      window.review.login("a@example.com", "password123"),
    );
    await first.evaluate(() => {
      window.pendingLogout = window.review.logout();
    });
    await waitForHeld("logout");
    await second.evaluate(() => {
      window.pendingAuth = window.review.login("b@example.com", "password123");
    });
    await assertQueued(second);
    assert.equal(
      seen.filter((call) => call.path === "/api/auth/login").length,
      1,
    );
    held.get("logout")();
    await first.evaluate(() => window.pendingLogout);
    await second.evaluate(() => window.pendingAuth);
    assert.equal(
      await second.evaluate(() => window.review.useSession.getState().user?.id),
      "user-B",
    );
    assert.deepEqual(
      await second.evaluate(() => window.review.api("/servers")),
      { owner: "user-B" },
    );
    console.log(
      "PASS late logout / new login across tabs: B cookie and UI agree",
    );
    await context.close();
  }
  {
    mode = "hold-login";
    held.clear();
    seen.length = 0;
    const context = await browser.newContext();
    const page = await context.newPage();
    await load(page);
    await page.evaluate(() => {
      window.pendingAuth = window.review
        .login("a@example.com", "password123")
        .catch((error) => error.name);
    });
    await waitForHeld("login");
    await page.evaluate(() => {
      window.pendingLogout = window.review
        .login("b@example.com", "password123")
        .then(() => undefined);
    });
    await assertQueued(page);
    assert.equal(
      seen.filter((call) => call.path === "/api/auth/login").length,
      1,
    );
    held.get("login")();
    assert.equal(await page.evaluate(() => window.pendingAuth), "AbortError");
    await page.evaluate(() => window.pendingLogout);
    assert.equal(
      await page.evaluate(() => window.review.useSession.getState().user?.id),
      "user-B",
    );
    assert.deepEqual(await page.evaluate(() => window.review.api("/servers")), {
      owner: "user-B",
    });
    console.log("PASS superseded login: latest B intent owns the cookie");
    await context.close();
  }
  {
    mode = "hold-login";
    held.clear();
    seen.length = 0;
    const context = await browser.newContext();
    const page = await context.newPage();
    await load(page);
    await page.evaluate(() => {
      window.pendingAuth = window.review
        .login("a@example.com", "password123")
        .catch((error) => error.name);
    });
    await waitForHeld("login");
    await page.evaluate(() => {
      window.pendingLogout = window.review.logout();
    });
    await assertQueued(page);
    held.get("login")();
    await page.evaluate(() => window.pendingAuth);
    await page.evaluate(() => window.pendingLogout);
    assert.equal(
      await page.evaluate(() => window.review.useSession.getState().user),
      null,
    );
    assert.equal(
      await page.evaluate(() =>
        window.review.api("/servers").then(
          () => "authenticated",
          (error) => error.code,
        ),
      ),
      "unauthenticated",
    );
    await load(page);
    assert.equal(
      await page.evaluate(() => window.review.useSession.getState().user),
      null,
    );
    console.log(
      "PASS logout during login: anonymous after response and reload",
    );
    await context.close();
  }
  for (const operation of ["login", "register"]) {
    mode = "";
    held.clear();
    seen.length = 0;
    const context = await browser.newContext();
    const page = await context.newPage();
    const other = await context.newPage();
    await load(page);
    await load(other);
    await other.evaluate(() =>
      window.review.login("a@example.com", "password123"),
    );
    const observed = await page.evaluate(async (operation) => {
      const states = [];
      const off = window.review.useSession.subscribe((state) =>
        states.push(state.user?.id ?? null),
      );
      const user =
        operation === "login"
          ? await window.review.login("b@example.com", "password123")
          : await window.review.register("b@example.com", "password123", "B");
      off();
      return { id: user.id, states };
    }, operation);
    assert.deepEqual(observed, { id: "user-B", states: ["user-B"] });
    assert.deepEqual(await page.evaluate(() => window.review.api("/servers")), {
      owner: "user-B",
    });
    console.log(
      `PASS explicit ${operation} from anonymous tab with foreign A cookie: B signs in`,
    );
    await context.close();
  }
  console.log(`Browser: Chromium ${browser.version()}`);
} finally {
  await browser.close();
  await server.close();
  await rm(cache, { recursive: true, force: true });
}
