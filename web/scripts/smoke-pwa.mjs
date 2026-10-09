// Production-build smoke against the real nginx: cache headers and a second
// deployment, real Chromium manifest/SW, emulated mobile UI. Install-dialog
// events, the gateway, the page going to the background and the notification
// tap are synthetic (OS installation and system notifications remain device
// checks).
/* global process, URL, window, navigator, caches, document, console, Event,
   fetch, localStorage, getComputedStyle, self, NotificationEvent, setTimeout,
   ServiceWorkerRegistration */
import assert from "node:assert/strict";
import {
  mkdtemp,
  readFile,
  rename,
  rm,
  stat,
  utimes,
  writeFile,
} from "node:fs/promises";
import { createServer, request as forward } from "node:http";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { chromium, devices } from "playwright";

const origin = new URL(process.env.GELABBER_PWA_URL ?? "http://127.0.0.1:4173");
assert(
  ["127.0.0.1", "localhost", "[::1]"].includes(origin.hostname),
  "PWA smoke must use a loopback server",
);
assert(
  !origin.username &&
    !origin.password &&
    origin.pathname === "/" &&
    !origin.search &&
    !origin.hash,
);
// The directory the server under test serves. The second-deployment check
// swaps the build in it and puts the original back.
const dist = process.env.GELABBER_PWA_DIST
  ? resolve(process.env.GELABBER_PWA_DIST)
  : fileURLToPath(new URL("../dist", import.meta.url));
// Playwright's default headless shell has no notifications, so the bundled
// full Chromium is used in headless mode unless an executable is named.
// Headless Chromium still hands notifications to the desktop's notification
// service; without the session bus the test ones stay inside the browser.
const launch = {
  ...(process.env.GELABBER_PWA_BROWSER_EXECUTABLE
    ? { executablePath: process.env.GELABBER_PWA_BROWSER_EXECUTABLE }
    : { channel: "chromium" }),
  headless: true,
  env: { ...process.env, DBUS_SESSION_BUS_ADDRESS: "disabled:" },
};
const browser = await chromium.launch(launch);
const USER = "00000000-0000-4000-8000-000000000006";
const url = (path) => new URL(path, origin).href;
const dm = (id, peer, name) => ({
  id,
  kind: "dm",
  created_at: "2026-01-01T00:00:00Z",
  peer: { id: peer, name, avatar_url: null },
});
const ADA = dm(
  "00000000-0000-4000-8000-0000000000d1",
  "00000000-0000-4000-8000-0000000000a1",
  "Ada",
);
const GRACE = dm(
  "00000000-0000-4000-8000-0000000000d2",
  "00000000-0000-4000-8000-0000000000a2",
  "Grace",
);
const LINUS = dm(
  "00000000-0000-4000-8000-0000000000d3",
  "00000000-0000-4000-8000-0000000000a3",
  "Linus",
);

async function contextFor(
  device,
  { standalone = false, authenticated = false, themes = {}, dms = [] } = {},
) {
  const context = await browser.newContext({
    ...device,
    serviceWorkers: "allow",
  });
  // Isolated fixture responses; this smoke neither connects to a database nor
  // creates real accounts. Real browser and worker behavior remains unmocked.
  let signedIn = authenticated;
  await context.route("**/api/**", async (route) => {
    const path = new URL(route.request().url()).pathname;
    if (path === "/api/auth/logout") signedIn = false;
    const user = signedIn
      ? {
          id: USER,
          email: "pwa@example.test",
          name: "PWA",
          avatar_url: null,
        }
      : null;
    const json =
      path === "/api/auth/session"
        ? { user, csrf_token: "a".repeat(64) }
        : path === "/api/dms"
          ? dms
          : path === "/api/servers" || path === "/api/messages/unread"
            ? []
            : path === "/api/me/themes"
              ? themes
              : /^\/api\/channels\/[^/]+\/messages$/.test(path)
                ? { messages: [], has_more: false }
                : (dms.find((row) => path === `/api/dms/${row.id}`) ?? {});
    await route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify(json),
    });
  });
  if (standalone) {
    await context.addInitScript(() => {
      const original = window.matchMedia.bind(window);
      window.matchMedia = (query) => {
        const media = original(query);
        if (query === "(display-mode: standalone)")
          Object.defineProperty(media, "matches", { value: true });
        return media;
      };
    });
  }
  return context;
}

// For page state that is only readable asynchronously: waitForFunction does
// not await an async predicate, it takes the pending promise for a yes.
async function until(check, message) {
  for (let waited = 0; waited < 15000; waited += 50) {
    if (await check()) return;
    await new Promise((resolve) => setTimeout(resolve, 50));
  }
  assert.fail(message);
}
const controlled = (page) =>
  page.waitForFunction(
    () => navigator.serviceWorker.controller !== null,
    null,
    { timeout: 15000 },
  );
const installHeading = (page) =>
  page.getByRole("heading", { name: "Gelabber als App" });
const installButton = (page) =>
  page.getByRole("button", { name: "App installieren", exact: true });
const offerInstallation = (page) =>
  page.evaluate(() => {
    window.pwaPromptCalls = 0;
    window.dispatchEvent(
      Object.assign(new Event("beforeinstallprompt", { cancelable: true }), {
        prompt: async () => {
          window.pwaPromptCalls++;
        },
        userChoice: Promise.resolve({ outcome: "accepted" }),
      }),
    );
  });

// Plain HTTP, as a browser asks for it. Without these headers an installed
// app keeps an old shell after a server upgrade and can end up blank.
const get = (path, headers = {}) =>
  fetch(url(path), {
    headers: { "accept-encoding": "gzip", ...headers },
    redirect: "manual",
  });

async function serving() {
  const html = await (await get("/")).text();
  for (const path of [
    "/",
    "/login",
    "/s/a/c/b",
    "/invite/code?from=link",
    "/index.html",
  ]) {
    const response = await get(path);
    assert.equal(response.status, 200, path);
    assert.match(response.headers.get("content-type"), /text\/html/, path);
    assert.equal(
      response.headers.get("cache-control"),
      "no-store",
      `${path}: no browser may keep the app shell`,
    );
    assert.equal(response.headers.get("content-encoding"), "gzip", path);
    assert.equal(await response.text(), html, path);
  }

  const assets = [...html.matchAll(/(?:src|href)="(\/assets\/[^"]+)"/g)].map(
    (match) => match[1],
  );
  assert(
    assets.some((asset) => asset.endsWith(".js")),
    "no script in shell",
  );
  assert(
    assets.some((asset) => asset.endsWith(".css")),
    "no styles in shell",
  );
  for (const asset of assets) {
    const response = await get(asset);
    assert.equal(response.status, 200, asset);
    assert.equal(
      response.headers.get("cache-control"),
      "public, max-age=31536000, immutable",
      asset,
    );
    assert.equal(response.headers.get("content-encoding"), "gzip", asset);
    assert.match(
      response.headers.get("content-type"),
      asset.endsWith(".js") ? /javascript/ : /text\/css/,
      asset,
    );
    await response.arrayBuffer();
  }
  for (const missing of [
    "/assets/index-absent.js",
    "/assets/index-absent.css",
    "/icons/absent.png",
    "/audio/absent.wasm",
    "/images/absent.png",
  ]) {
    const response = await get(missing);
    assert.equal(
      response.status,
      404,
      `${missing}: a missing file must not be answered with the app shell`,
    );
    assert.doesNotMatch(
      response.headers.get("cache-control") ?? "",
      /immutable|max-age/,
      `${missing}: the 404 must not be cacheable for a year`,
    );
    await response.arrayBuffer();
  }

  const manifest = await get("/manifest.webmanifest");
  assert.match(
    manifest.headers.get("content-type"),
    /application\/manifest\+json/,
  );
  assert.equal(manifest.headers.get("cache-control"), "no-cache");
  const worker = await get("/sw.js");
  assert.match(worker.headers.get("content-type"), /javascript/);
  assert.equal(worker.headers.get("cache-control"), "no-cache");
  const offline = await get("/offline.html");
  assert.equal(offline.headers.get("cache-control"), "no-cache");

  // The tab icon used to be a 2 MB banner.
  const icon = html.match(/<link\s+rel="icon"[^>]*\shref="([^"]+)"/)?.[1];
  assert(icon, "no favicon in the shell");
  const iconResponse = await get(icon);
  assert.equal(iconResponse.status, 200, icon);
  assert.match(iconResponse.headers.get("content-type"), /image\/png/);
  assert((await iconResponse.arrayBuffer()).byteLength < 64 * 1024, icon);
  assert.match(
    html,
    /<meta\s+name="viewport"\s+content="[^"]*viewport-fit=cover[^"]*"/,
    "safe-area insets need viewport-fit=cover",
  );
  assert.match(
    html,
    /<link\s+rel="manifest"[^>]*\scrossorigin="use-credentials"/,
    "the built shell must ask for the manifest with cookies",
  );
  console.log(
    "PASS: app shell never stored (no-store), hashed assets immutable and gzip, missing files 404, small favicon",
  );
}

async function androidInstallAndOffline() {
  const android = await contextFor(devices["Pixel 7"]);
  await android.addInitScript(() =>
    localStorage.setItem("gelabber.theme", "light"),
  );
  const page = await android.newPage();
  const errors = [];
  page.on("pageerror", (error) => errors.push(error.message));
  await page.goto(url("/login"));
  await installHeading(page).waitFor();
  await controlled(page);
  const cdp = await android.newCDPSession(page);
  const manifest = await cdp.send("Page.getAppManifest");
  assert.deepEqual(
    manifest.errors,
    [],
    "Chrome must accept the served app manifest",
  );
  const installability = await cdp.send("Page.getInstallabilityErrors");
  assert.deepEqual(
    installability.installabilityErrors.filter(
      (error) => error.errorId !== "in-incognito",
    ),
    [],
    "Only the isolated test profile may prevent installation; app criteria must pass",
  );
  const app = JSON.parse(manifest.data);
  assert.equal(app.display, "standalone");
  assert.equal(app.id, "/");
  for (const icon of app.icons) {
    const response = await android.request.get(url(icon.src));
    assert.equal(response.status(), 200);
    assert.match(response.headers()["content-type"], /image\/png/);
  }
  // Before the browser offers a prompt there is no button, only the menu path.
  assert.equal(await installButton(page).count(), 0);
  await page.getByText("Wähle im Browsermenü", { exact: false }).waitFor();
  await offerInstallation(page);
  await installButton(page).click();
  await page
    .getByRole("status")
    .filter({ hasText: "Installation bestätigt" })
    .waitFor();
  assert.equal(await page.evaluate(() => window.pwaPromptCalls), 1);
  await page.evaluate(() => window.dispatchEvent(new Event("appinstalled")));
  await page
    .getByRole("status")
    .filter({ hasText: "bereits als App" })
    .waitFor();
  assert.equal(await installButton(page).count(), 0);
  const cacheEntries = await page.evaluate(async () => {
    const names = (await caches.keys()).filter((name) =>
      name.startsWith("gelabber-pwa-offline-"),
    );
    return Promise.all(
      names.map(async (name) =>
        (await (await caches.open(name)).keys()).map(
          (request) => new URL(request.url).pathname,
        ),
      ),
    );
  });
  assert.deepEqual(cacheEntries, [["/offline.html"]]);

  // Offline at a deep link with a query string. The offline page stands in
  // at that address, and retrying must ask for exactly that address again.
  const deepLink = url("/s/offline/c/offline?from=notification");
  const offlineHeading = page.getByRole("heading", {
    name: "Gerade keine Verbindung",
  });
  // setOffline() fails the requests; the override is what the page sees as
  // navigator.onLine and as `offline`/`online` events.
  const connection = async (offline) => {
    await android.setOffline(offline);
    await cdp.send("Network.overrideNetworkState", {
      offline,
      latency: 0,
      downloadThroughput: -1,
      uploadThroughput: -1,
    });
  };
  await connection(true);
  await page.goto(deepLink);
  await offlineHeading.waitFor();
  assert.equal(page.url(), deepLink);
  // The offline page wears the theme the app last painted (light here).
  assert.deepEqual(
    await page.evaluate(() => [
      getComputedStyle(document.documentElement).backgroundColor,
      document.querySelector('meta[name="theme-color"]').content,
    ]),
    ["rgb(248, 245, 240)", "#f8f5f0"],
  );
  const [retried] = await Promise.all([
    page.waitForRequest((request) => request.isNavigationRequest()),
    page.waitForEvent("load"),
    page.getByRole("link", { name: "Erneut verbinden" }).click(),
  ]);
  assert.equal(retried.url(), deepLink, "retry must keep the deep link");
  await offlineHeading.waitFor();
  assert.equal(page.url(), deepLink);
  // Back online the page reloads by itself; the app then sends the anonymous
  // fixture user to the login page and remembers where they wanted to go.
  // (The emulated state does not survive a navigation, so the page that is
  // showing now is told it is offline before the connection returns.)
  await connection(true);
  assert.equal(await page.evaluate(() => navigator.onLine), false);
  await connection(false);
  await page.getByRole("heading", { name: "Anmelden", exact: true }).waitFor();
  assert.equal(
    new URL(page.url()).searchParams.get("redirect"),
    "/s/offline/c/offline?from=notification",
  );
  assert.deepEqual(errors, []);
  console.log(
    "PASS: production manifest/icons, real service worker, Android install-event UI, offline deep link kept on retry, automatic reload when back online",
  );
  await android.close();
}

// Behind an access proxy that lets nothing through without its cookie
// (Authelia, oauth2-proxy, Cloudflare Access). A browser asks for a manifest
// without cookies unless the link says `use-credentials`. The proxy then
// answers with its own login, the browser has no manifest and never offers
// installation. The stand-in sends a visitor without its cookie through a
// sign-in address and passes every request that carries the cookie on to the
// server under test.
async function manifestBehindAccessProxy() {
  const refused = [];
  const proxy = createServer((request, response) => {
    if (request.url === "/access") {
      response.writeHead(302, {
        "set-cookie": "access=granted; Path=/; HttpOnly; SameSite=Lax",
        location: "/login",
      });
      response.end();
      return;
    }
    if (!/(?:^|;\s*)access=granted(?:;|$)/.test(request.headers.cookie ?? "")) {
      refused.push(request.url);
      response.writeHead(302, { location: "/access" });
      response.end();
      return;
    }
    request.pipe(
      forward(
        new URL(request.url, origin),
        { method: request.method, headers: request.headers },
        (answer) => {
          response.writeHead(answer.statusCode, answer.headers);
          answer.pipe(response);
        },
      ),
    );
  });
  await new Promise((resolve) => proxy.listen(0, "127.0.0.1", resolve));
  const context = await contextFor(devices["Pixel 7"]);
  try {
    const page = await context.newPage();
    await page.goto(`http://127.0.0.1:${proxy.address().port}/login`);
    await installHeading(page).waitFor();
    await controlled(page);
    const cdp = await context.newCDPSession(page);
    const manifest = await cdp.send("Page.getAppManifest");
    assert.deepEqual(manifest.errors, []);
    assert.equal(
      manifest.data ? JSON.parse(manifest.data).id : undefined,
      "/",
      "the browser must get the manifest through the access proxy",
    );
    const installability = await cdp.send("Page.getInstallabilityErrors");
    assert.deepEqual(
      installability.installabilityErrors.filter(
        (error) => error.errorId !== "in-incognito",
      ),
      [],
    );
    assert.deepEqual(
      refused,
      ["/login"],
      "after the first visit every request must carry the proxy's cookie",
    );
  } finally {
    await context.close();
    proxy.close();
    proxy.closeAllConnections();
  }
  console.log(
    "PASS: manifest and installability behind an access proxy that requires its cookie on every path",
  );
}

async function installPanelPlacement() {
  const ios = await contextFor(devices["iPhone 13"]);
  const iphone = await ios.newPage();
  await iphone.goto(url("/login"));
  await iphone
    .getByText("Öffne Gelabber in Safari.", { exact: true })
    .waitFor();
  await iphone
    .getByText("Tippe auf „Teilen“, dann auf „Zum Home-Bildschirm“.", {
      exact: true,
    })
    .waitFor();
  assert.equal(await installButton(iphone).count(), 0);
  assert.equal(
    await iphone.evaluate(
      () => document.documentElement.scrollWidth > window.innerWidth,
    ),
    false,
  );
  console.log(
    "PASS: iPhone instruction UI at mobile width (Chromium emulation, not physical Safari acceptance)",
  );
  await ios.close();

  // Inside the installed app there is nothing left to install.
  const installed = await contextFor(devices["Pixel 7"], {
    standalone: true,
    authenticated: true,
  });
  const standalonePage = await installed.newPage();
  await standalonePage.goto(url("/settings"));
  await standalonePage
    .getByRole("button", { name: "Darstellung", exact: true })
    .waitFor();
  assert.equal(
    await standalonePage
      .getByRole("button", { name: "App", exact: true })
      .count(),
    0,
  );
  await installed.close();
  const installedAnonymous = await contextFor(devices["Pixel 7"], {
    standalone: true,
  });
  const standaloneLogin = await installedAnonymous.newPage();
  await standaloneLogin.goto(url("/login"));
  await standaloneLogin
    .getByRole("heading", { name: "Anmelden", exact: true })
    .waitFor();
  assert.equal(await installHeading(standaloneLogin).count(), 0);
  await installedAnonymous.close();
  console.log("PASS: standalone mode shows no install UI");

  // A desktop browser gets the panel on the login page only once it offers a
  // prompt, and without the phone wording.
  const desktop = await contextFor(devices["Desktop Chrome"]);
  const desktopPage = await desktop.newPage();
  await desktopPage.goto(url("/login"));
  await desktopPage
    .getByRole("heading", { name: "Anmelden", exact: true })
    .waitFor();
  assert.equal(await installHeading(desktopPage).count(), 0);
  await offerInstallation(desktopPage);
  await installButton(desktopPage).waitFor();
  assert.equal(await desktopPage.getByText("Startbildschirm").count(), 0);
  await desktop.close();
  console.log("PASS: desktop login offers installation only with a prompt");

  const settings = await contextFor(devices["Pixel 7"], {
    authenticated: true,
  });
  const settingsPage = await settings.newPage();
  await settingsPage.goto(url("/settings"));
  await settingsPage.getByRole("button", { name: "App", exact: true }).click();
  await installHeading(settingsPage).waitFor();
  assert.equal(
    await settingsPage.evaluate(
      () => document.documentElement.scrollWidth > window.innerWidth,
    ),
    false,
  );
  await settings.close();
  console.log("PASS: install panel reachable from authenticated settings");
}

async function themeColour() {
  const colours = (page) =>
    page.evaluate(() => ({
      meta: document.querySelector('meta[name="theme-color"]').content,
      page: getComputedStyle(document.documentElement)
        .getPropertyValue("--lr-bg")
        .trim(),
    }));
  const follows = (page, colour) =>
    page.waitForFunction(
      (expected) =>
        document.querySelector('meta[name="theme-color"]').content === expected,
      colour,
    );

  // Device preference of a signed-out user: light, then following the system.
  const light = await contextFor(devices["Pixel 7"]);
  await light.addInitScript(() =>
    localStorage.setItem("gelabber.theme", "light"),
  );
  const lightPage = await light.newPage();
  await lightPage.goto(url("/login"));
  await lightPage
    .getByRole("heading", { name: "Anmelden", exact: true })
    .waitFor();
  assert.deepEqual(await colours(lightPage), {
    meta: "#f8f5f0",
    page: "#f8f5f0",
  });
  await light.close();

  const system = await contextFor({
    ...devices["Pixel 7"],
    colorScheme: "light",
  });
  await system.addInitScript(() =>
    localStorage.setItem("gelabber.theme", "system"),
  );
  const systemPage = await system.newPage();
  await systemPage.goto(url("/login"));
  await systemPage
    .getByRole("heading", { name: "Anmelden", exact: true })
    .waitFor();
  assert.equal((await colours(systemPage)).meta, "#f8f5f0");
  await systemPage.emulateMedia({ colorScheme: "dark" });
  await follows(systemPage, "#17191a");
  await system.close();

  // Account theme: a custom palette from the server, then the same palette
  // from the paint cache with the bundle blocked, i.e. before React starts.
  const custom = {
    version: 1,
    id: "custom-00000000-0000-4000-8000-0000000000aa",
    name: "Smoke",
    mode: "light",
    style: "clear",
    colors: {
      background: "#d8e6f0",
      panel: "#cfdde8",
      surface: "#c3d2de",
      rail: "#c9d8e3",
      text: "#10202c",
      muted: "#40525f",
      accent: "#1f5f8b",
      border: "#aebfcc",
    },
  };
  const account = await contextFor(devices["Pixel 7"], {
    authenticated: true,
    themes: {
      version: 1,
      revision: 3,
      active: custom.id,
      customThemes: [custom],
    },
  });
  const accountPage = await account.newPage();
  await accountPage.goto(url("/settings"));
  await follows(accountPage, "#d8e6f0");
  assert.equal((await colours(accountPage)).page, "#d8e6f0");
  await account.route("**/assets/*.js", (route) => route.abort());
  await accountPage.reload();
  assert.deepEqual(await colours(accountPage), {
    meta: "#d8e6f0",
    page: "#d8e6f0",
  });
  await account.close();
  console.log(
    "PASS: theme-color follows light, system and a custom account theme, also before the app starts",
  );
}

// A gateway that acknowledges subscriptions and delivers what the test says.
async function gatewayFor(context) {
  const sockets = new Set();
  const subscribed = new Set();
  const heads = new Map();
  await context.routeWebSocket("**/ws", (socket) => {
    sockets.add(socket);
    socket.onMessage((raw) => {
      const frame = JSON.parse(raw);
      if (frame.op !== "s") return;
      subscribed.add(frame.c);
      socket.send(
        JSON.stringify({
          op: "ok",
          s: frame.s,
          c: frame.c,
          n: heads.get(frame.c) ?? 0,
        }),
      );
    });
  });
  return {
    subscribed: (...conversations) =>
      until(
        () => conversations.every((row) => subscribed.has(row.id)),
        "the app did not subscribe to its conversations",
      ),
    /** Delivers a message to every window and returns its id. */
    message(conversation, content) {
      const n = (heads.get(conversation.id) ?? 0) + 1;
      heads.set(conversation.id, n);
      const id = `00000000-0000-4000-8000-${String(n).padStart(8, "0")}${conversation.id.slice(-4)}`;
      const frame = JSON.stringify({
        op: "e",
        t: "c",
        s: conversation.id,
        c: conversation.id,
        n,
        d: {
          id,
          channel_id: conversation.id,
          author: conversation.peer,
          content,
          created_at: new Date().toISOString(),
          edited_at: null,
          attachments: [],
        },
      });
      for (const socket of sockets) socket.send(frame);
      return id;
    },
  };
}

// From a gateway message to a notification on a phone, and what takes it away
// again. Headless Chromium never hides a page, so the page is told it is in
// the background; the gateway is the stand-in above. The rest is the real
// app: its decision, the worker registration and the browser's own list.
async function messageNotifications() {
  const context = await contextFor(devices["Pixel 7"], {
    authenticated: true,
    dms: [ADA, GRACE, LINUS],
  });
  await context.grantPermissions(["notifications"], { origin: origin.origin });
  const gateway = await gatewayFor(context);
  await context.addInitScript(() => {
    // "Benachrichtigung, wenn der Tab im Hintergrund ist" switched on.
    localStorage.setItem(
      "gelabber.media",
      JSON.stringify({ state: { desktopNotify: true }, version: 0 }),
    );
    let hidden = false;
    Object.defineProperty(document, "hidden", { get: () => hidden });
    Object.defineProperty(document, "visibilityState", {
      get: () => (hidden ? "hidden" : "visible"),
    });
    window.pwaBackground = (value) => {
      hidden = value;
      document.dispatchEvent(new Event("visibilitychange"));
    };
    // How often this window asks the browser for a notification.
    window.pwaAsked = 0;
    const show = ServiceWorkerRegistration.prototype.showNotification;
    ServiceWorkerRegistration.prototype.showNotification = function (...args) {
      window.pwaAsked++;
      return show.apply(this, args);
    };
  });
  const errors = [];
  const page = await context.newPage();
  page.on("pageerror", (error) => errors.push(error.message));
  await page.goto(url(`/d/${ADA.id}`));
  await page.getByTestId("message-pane").waitFor();
  await controlled(page);
  await gateway.subscribed(ADA, GRACE, LINUS);
  const shown = () =>
    page.evaluate(async () => {
      const registration = await navigator.serviceWorker.getRegistration();
      return (await registration.getNotifications())
        .map(({ title, body, tag, data, silent, renotify }) => ({
          title,
          body,
          tag,
          data,
          silent,
          renotify,
        }))
        .sort((a, b) => a.tag.localeCompare(b.tag));
    });
  const tags = async () =>
    (await shown()).map((notification) => notification.tag);
  const left = (count) =>
    until(
      async () => (await shown()).length === count,
      `expected ${count} notification(s) on screen`,
    );

  // In the foreground a message is a toast in the app, not a notification.
  gateway.message(GRACE, "Bist du da?");
  await page.getByText("Bist du da?").first().waitFor();
  await page.waitForTimeout(500);
  assert.deepEqual(await shown(), []);

  // On a phone (this is one, by its user agent) the notification sounds and
  // vibrates as the device is set, and again for a later message of the
  // conversation: nothing else tells of a message while the app is not in
  // front.
  await page.evaluate(() => window.pwaBackground(true));
  const hallo = gateway.message(ADA, "Hallo");
  const nochDa = gateway.message(GRACE, "Noch da?");
  await left(2);
  assert.deepEqual(await shown(), [
    {
      title: "Ada · Ada",
      body: "Hallo",
      tag: `gelabber:${USER}:${ADA.id}`,
      data: { path: `/d/${ADA.id}`, user: USER, message: hallo },
      silent: false,
      renotify: true,
    },
    {
      title: "Grace · Grace",
      body: "Noch da?",
      tag: `gelabber:${USER}:${GRACE.id}`,
      data: { path: `/d/${GRACE.id}`, user: USER, message: nochDa },
      silent: false,
      renotify: true,
    },
  ]);

  // A second app window starts, as after a tap when the phone had discarded
  // the app, and is reloaded. Each time it learns who is signed in and must
  // leave both notifications where they are.
  const both = [`gelabber:${USER}:${ADA.id}`, `gelabber:${USER}:${GRACE.id}`];
  const second = await context.newPage();
  second.on("pageerror", (error) => errors.push(error.message));
  await second.goto(url("/"));
  await second.getByRole("heading", { name: "Hallo PWA" }).waitFor();
  await second.waitForTimeout(500);
  assert.deepEqual(
    await tags(),
    both,
    "a starting app window must not close the notifications on screen",
  );
  await second.goto(url("/settings"));
  const signOut = second.getByRole("button", { name: "Abmelden", exact: true });
  await signOut.waitFor();
  await second.waitForTimeout(500);
  assert.deepEqual(await tags(), both);

  // Back through the app switcher instead of the tap: the conversation that
  // is on screen loses its notification, the other one keeps it.
  await page.evaluate(() => window.pwaBackground(false));
  await left(1);
  await page.waitForTimeout(500);
  assert.deepEqual(await tags(), [`gelabber:${USER}:${GRACE.id}`]);

  // Two windows of the app, as with the browser tab that stays behind after
  // installing. The one in front shows Ada's conversation, the one in the
  // background hears the same messages and decides from its own visibility.
  // (A toast in the second window first: it is connected and listening.)
  gateway.message(GRACE, "Hörst du mich?");
  await second.getByText("Hörst du mich?").first().waitFor();
  await second.evaluate(() => window.pwaBackground(true));
  gateway.message(ADA, "Liest du das gerade?");
  await page.getByText("Liest du das gerade?").first().waitFor();
  // What it does about Grace's message comes after what it did about Ada's.
  gateway.message(GRACE, "Und du?");
  await until(
    async () => (await shown()).some(({ body }) => body === "Und du?"),
    "a window in the background must still notify about other conversations",
  );
  assert.deepEqual(
    await tags(),
    [`gelabber:${USER}:${GRACE.id}`],
    "a conversation that is on screen in another window must not raise a notification",
  );

  // Both windows in the background, as with the phone in a pocket. Each of
  // them hears Linus, and the phone must sound once.
  const asked = async () =>
    (await page.evaluate(() => window.pwaAsked)) +
    (await second.evaluate(() => window.pwaAsked));
  const askedBefore = await asked();
  await page.evaluate(() => window.pwaBackground(true));
  const linus = gateway.message(LINUS, "Zwei Fenster");
  await until(
    async () => (await shown()).some(({ data }) => data.message === linus),
    "two windows in the background must still announce a message",
  );
  await page.waitForTimeout(500);
  assert.equal(
    (await asked()) - askedBefore,
    1,
    "two windows in the background must announce a message once",
  );
  await page.evaluate(() => window.pwaBackground(false));
  await second.evaluate(() => window.pwaBackground(false));

  // Signing out in any window takes the account's notifications along.
  await signOut.click();
  await second
    .getByRole("heading", { name: "Anmelden", exact: true })
    .waitFor();
  await left(0);
  assert.deepEqual(errors, []);
  await context.close();
  console.log(
    "PASS: gateway message in the background becomes an audible notification through the worker; a starting window keeps them, the conversation on screen and a sign-out withdraw them; a window in the background stays quiet about what another one shows, two of them announce a message once (scripted gateway and visibility)",
  );
}

async function notificationTap() {
  const context = await contextFor(devices["Pixel 7"], {
    authenticated: true,
    dms: [ADA, GRACE],
  });
  await context.grantPermissions(["notifications"], { origin: origin.origin });
  const page = await context.newPage();
  const errors = [];
  page.on("pageerror", (error) => errors.push(error.message));
  await page.goto(url("/settings"));
  await controlled(page);
  // What src/pwa/notifications.ts sends through the worker's registration.
  const show = (user, conversation) =>
    page.evaluate(
      async ({ user, conversation }) => {
        const registration = await navigator.serviceWorker.getRegistration();
        await registration.showNotification("Ada · DM", {
          body: "Hallo",
          silent: false,
          tag: `gelabber:${user}:${conversation}`,
          renotify: true,
          icon: "/icons/icon-192.png",
          data: { path: `/d/${conversation}`, user, message: "message-1" },
        });
        return (await registration.getNotifications()).map(
          (notification) => notification.tag,
        );
      },
      { user, conversation },
    );
  // A script-made event is not trusted: the browser refuses its waitUntil()
  // and the window focus. The rest of the worker's handler runs for real:
  // closing the notification, finding the app window, telling it where to go.
  const tap = () =>
    context.serviceWorkers()[0].evaluate(async () => {
      const [notification] = await self.registration.getNotifications();
      self.dispatchEvent(
        new NotificationEvent("notificationclick", { notification }),
      );
    });
  const shown = () =>
    page.evaluate(async () => {
      const registration = await navigator.serviceWorker.getRegistration();
      return (await registration.getNotifications()).length;
    });

  const conversation = ADA.id;
  assert.deepEqual(await show(USER, conversation), [
    `gelabber:${USER}:${conversation}`,
  ]);
  await page.evaluate(() => {
    window.pwaSamePage = true;
  });
  await tap();
  await page.waitForURL(url(`/d/${conversation}`));
  await page.getByTestId("message-pane").waitFor();
  assert.equal(
    await page.evaluate(() => window.pwaSamePage),
    true,
    "a tap must change the route inside the running app, not reload it",
  );
  assert.equal(await shown(), 0, "a tapped notification must be closed");

  // Shown for another account: the tap leads nowhere.
  await show("someone-else", GRACE.id);
  await tap();
  await until(
    async () => (await shown()) === 0,
    "a tapped notification of another account must be closed too",
  );
  await page.waitForTimeout(300);
  assert.equal(page.url(), url(`/d/${conversation}`));
  assert.deepEqual(errors, []);
  await context.close();
  console.log(
    "PASS: notification through the worker registration, tap takes the running app to the conversation (synthetic tap event)",
  );
}

// The app after a server upgrade. A real browser profile with its HTTP cache
// and no request routing (routing would switch the cache off). The first
// build's shell is given an old date, as on a server that has been running
// for weeks: a browser that is allowed to keep it then treats it as fresh for
// days and never asks for the new one.
//
// Going back to the app stands for every load that takes a stored copy
// without asking the server, whatever its age: Chromium loads a tab restored
// at browser start and a discarded tab coming back the same way. Under
// `no-cache` this step starts the old client. (Playwright runs Chromium
// without the back/forward cache, so going back builds the document anew
// instead of reviving the page that was left.)
async function secondDeployment() {
  const indexFile = join(dist, "index.html");
  const html = await readFile(indexFile, "utf8");
  assert.equal(
    await (await get("/index.html")).text(),
    html,
    `the server at ${origin.origin} does not serve ${dist}; set GELABBER_PWA_DIST`,
  );
  const entry = html.match(/<script[^>]*\ssrc="\/assets\/([^"]+\.js)"/)?.[1];
  assert(entry, "no entry script in the shell");
  const nextEntry = entry.replace(/\.js$/, "-next.js");
  const { atime, mtime } = await stat(indexFile);
  const weeksAgo = new Date(Date.now() - 30 * 24 * 60 * 60 * 1000);
  const profile = await mkdtemp(join(tmpdir(), "gelabber-pwa-smoke-"));
  const entryOf = (page) =>
    page.evaluate(
      () =>
        new URL(document.querySelector('script[type="module"]').src).pathname,
    );
  const started = (page) =>
    page.waitForFunction(
      () => document.getElementById("root").childElementCount > 0,
    );
  const failures = [];
  const watched = (page) => {
    page.on("console", (message) => {
      if (message.type() === "error") failures.push(message.text());
    });
    return page;
  };
  // Another site, to leave the app for and come back from.
  const elsewhere = createServer((_request, response) => {
    response.writeHead(200, {
      "content-type": "text/html",
      "cache-control": "no-store",
    });
    response.end("<!doctype html><title>Elsewhere</title>");
  });
  await new Promise((resolve) => elsewhere.listen(0, "127.0.0.1", resolve));
  let context;
  try {
    await utimes(indexFile, weeksAgo, weeksAgo);
    context = await chromium.launchPersistentContext(profile, launch);
    let page = watched(await context.newPage());
    await page.goto(url("/login"));
    await started(page);
    await controlled(page);
    assert.equal(await entryOf(page), `/assets/${entry}`);
    await page.goto(`http://127.0.0.1:${elsewhere.address().port}/`);

    // Deploy: same code under a new hashed name, the old file is gone.
    await rename(join(dist, "assets", entry), join(dist, "assets", nextEntry));
    await writeFile(indexFile, html.replace(entry, nextEntry));

    await page.goBack();
    assert.equal(page.url(), url("/login"));
    assert.equal(
      await entryOf(page),
      `/assets/${nextEntry}`,
      "going back to the app after a deployment must load the new shell, not a copy the browser kept",
    );
    await started(page);
    await context.close();

    context = await chromium.launchPersistentContext(profile, launch);
    page = watched(await context.newPage());
    await page.goto(url("/login"));
    assert.equal(
      await entryOf(page),
      `/assets/${nextEntry}`,
      "the next launch after a deployment must load the new shell",
    );
    await started(page);
    await page.reload();
    await started(page);
    assert.equal(await entryOf(page), `/assets/${nextEntry}`);
    assert.deepEqual(
      failures.filter((text) => /module script|MIME/i.test(text)),
      [],
    );
    // A tab that still runs the old shell gets a plain 404 for its chunk.
    assert.equal((await get(`/assets/${entry}`)).status, 404);
  } finally {
    await context?.close().catch(() => {});
    elsewhere.close();
    elsewhere.closeAllConnections();
    await rename(
      join(dist, "assets", nextEntry),
      join(dist, "assets", entry),
    ).catch(() => {});
    await writeFile(indexFile, html);
    await utimes(indexFile, atime, mtime);
    await rm(profile, { recursive: true, force: true });
  }
  console.log(
    "PASS: second deployment with new asset names is loaded when going back to the app, on the next launch and after reload; removed chunk is a 404",
  );
}

try {
  await serving();
  await androidInstallAndOffline();
  await manifestBehindAccessProxy();
  await installPanelPlacement();
  await themeColour();
  await messageNotifications();
  await notificationTap();
  await secondDeployment();
} finally {
  await browser.close();
}
