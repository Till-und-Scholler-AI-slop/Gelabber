// Production-build smoke against the real nginx: cache headers and a second
// deployment, real Chromium manifest/SW, emulated mobile UI. Install-dialog
// events and the notification tap are synthetic (OS installation and system
// notifications remain device checks).
/* global process, URL, window, navigator, caches, document, console, Event,
   fetch, localStorage, getComputedStyle, self, NotificationEvent */
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

async function contextFor(
  device,
  { standalone = false, authenticated = false, themes = {} } = {},
) {
  const context = await browser.newContext({
    ...device,
    serviceWorkers: "allow",
  });
  // Isolated fixture responses; this smoke neither connects to a database nor
  // creates real accounts. Real browser and worker behavior remains unmocked.
  await context.route("**/api/**", async (route) => {
    const path = new URL(route.request().url()).pathname;
    const user = authenticated
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
        : path === "/api/servers" || path === "/api/dms"
          ? []
          : path === "/api/me/themes"
            ? themes
            : {};
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
  const shell = await get("/");
  const html = await shell.text();
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
      "no-cache",
      `${path}: the app shell must be revalidated on every load`,
    );
    assert.equal(response.headers.get("content-encoding"), "gzip", path);
    assert.equal(await response.text(), html, path);
  }
  const unchanged = await get("/login", {
    "if-none-match": shell.headers.get("etag"),
  });
  assert.equal(unchanged.status, 304, "revalidation must be a cheap 304");

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
  console.log(
    "PASS: app shell revalidated (no-cache, 304), hashed assets immutable and gzip, missing files 404, small favicon",
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

async function notificationTap() {
  const context = await contextFor(devices["Pixel 7"], { authenticated: true });
  await context.grantPermissions(["notifications"], { origin: origin.origin });
  const page = await context.newPage();
  await page.goto(url("/settings"));
  await controlled(page);
  // What src/pwa/notifications.ts sends through the worker's registration.
  const show = (user, conversation) =>
    page.evaluate(
      async ({ user, conversation }) => {
        const registration = await navigator.serviceWorker.getRegistration();
        await registration.showNotification("Ada · DM", {
          body: "Hallo",
          silent: true,
          tag: `gelabber:${user}:${conversation}`,
          renotify: false,
          icon: "/icons/icon-192.png",
          data: { path: `/d/${conversation}`, user },
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

  const conversation = "00000000-0000-4000-8000-0000000000d1";
  assert.deepEqual(await show(USER, conversation), [
    `gelabber:${USER}:${conversation}`,
  ]);
  await page.evaluate(() => {
    window.pwaSamePage = true;
  });
  await tap();
  await page.waitForURL(url(`/d/${conversation}`));
  assert.equal(
    await page.evaluate(() => window.pwaSamePage),
    true,
    "a tap must change the route inside the running app, not reload it",
  );
  assert.equal(await shown(), 0, "a tapped notification must be closed");

  // Shown for another account: the tap leads nowhere.
  await show("someone-else", "00000000-0000-4000-8000-0000000000d2");
  await tap();
  await page.waitForFunction(async () => {
    const registration = await navigator.serviceWorker.getRegistration();
    return (await registration.getNotifications()).length === 0;
  });
  await page.waitForTimeout(300);
  assert.equal(page.url(), url(`/d/${conversation}`));
  await context.close();
  console.log(
    "PASS: notification through the worker registration, tap takes the running app to the conversation (synthetic tap event)",
  );
}

// The installed app after a server upgrade. A real browser profile with its
// HTTP cache and no request routing (routing would switch the cache off).
// The first build's shell is given an old date, as on a server that has been
// running for weeks: without `Cache-Control: no-cache` a browser then treats
// it as fresh for days and never asks for the new one.
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
  let context;
  try {
    await utimes(indexFile, weeksAgo, weeksAgo);
    context = await chromium.launchPersistentContext(profile, launch);
    let page = await context.newPage();
    await page.goto(url("/login"));
    await started(page);
    await controlled(page);
    assert.equal(await entryOf(page), `/assets/${entry}`);
    await context.close();

    // Deploy: same code under a new hashed name, the old file is gone.
    await rename(join(dist, "assets", entry), join(dist, "assets", nextEntry));
    await writeFile(indexFile, html.replace(entry, nextEntry));

    context = await chromium.launchPersistentContext(profile, launch);
    page = await context.newPage();
    const failures = [];
    page.on("console", (message) => {
      if (message.type() === "error") failures.push(message.text());
    });
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
    await rename(
      join(dist, "assets", nextEntry),
      join(dist, "assets", entry),
    ).catch(() => {});
    await writeFile(indexFile, html);
    await utimes(indexFile, atime, mtime);
    await rm(profile, { recursive: true, force: true });
  }
  console.log(
    "PASS: second deployment with new asset names is loaded on the next launch and after reload; removed chunk is a 404",
  );
}

try {
  await serving();
  await androidInstallAndOffline();
  await installPanelPlacement();
  await themeColour();
  await notificationTap();
  await secondDeployment();
} finally {
  await browser.close();
}
