// Production-build smoke: real Chromium manifest/SW, emulated mobile UI,
// synthetic install-dialog events (OS installation remains a device check).
/* global process, URL, window, navigator, caches, document, console, Event */
import assert from "node:assert/strict";
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
const browser = await chromium.launch({
  executablePath: process.env.GELABBER_PWA_BROWSER_EXECUTABLE || undefined,
  headless: true,
});

async function contextFor(
  device,
  { standalone = false, authenticated = false } = {},
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
          id: "00000000-0000-4000-8000-000000000006",
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

try {
  const android = await contextFor(devices["Pixel 7"]);
  const page = await android.newPage();
  const errors = [];
  page.on("pageerror", (error) => errors.push(error.message));
  await page.goto(new URL("/login", origin).href);
  await page.getByRole("heading", { name: "Gelabber als App" }).waitFor();
  await page.waitForFunction(
    () => navigator.serviceWorker.controller !== null,
    null,
    { timeout: 15000 },
  );
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
    const response = await android.request.get(new URL(icon.src, origin).href);
    assert.equal(response.status(), 200);
    assert.match(response.headers()["content-type"], /image\/png/);
  }
  assert.equal(
    (
      await android.request.get(new URL("/icons/absent.png", origin).href)
    ).status(),
    404,
  );
  const manifestResponse = await android.request.get(
    new URL("/manifest.webmanifest", origin).href,
  );
  assert.match(
    manifestResponse.headers()["content-type"],
    /application\/manifest\+json/,
  );
  const workerResponse = await android.request.get(
    new URL("/sw.js", origin).href,
  );
  assert.match(workerResponse.headers()["cache-control"], /no-cache/);
  await page.evaluate(() => {
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
  await page
    .getByRole("button", { name: "App installieren", exact: true })
    .click();
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
  assert.equal(
    await page
      .getByRole("button", { name: "App installieren", exact: true })
      .count(),
    0,
  );
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
  await android.setOffline(true);
  await page.goto(new URL("/s/offline/c/offline", origin).href);
  await page
    .getByRole("heading", { name: "Gerade keine Verbindung" })
    .waitFor();
  await android.setOffline(false);
  await page.getByRole("link", { name: "Erneut verbinden" }).click();
  await page.getByRole("heading", { name: "Anmelden", exact: true }).waitFor();
  assert.deepEqual(errors, []);
  console.log(
    "PASS: production manifest/icons, real service worker, Android install-event UI, offline navigation and reconnection",
  );
  await android.close();

  const ios = await contextFor(devices["iPhone 13"]);
  const iphone = await ios.newPage();
  await iphone.goto(new URL("/login", origin).href);
  await iphone
    .getByText("Öffne Gelabber in Safari.", { exact: true })
    .waitFor();
  await iphone
    .getByText("Tippe auf „Teilen“, dann auf „Zum Home-Bildschirm“.", {
      exact: true,
    })
    .waitFor();
  assert.equal(
    await iphone
      .getByRole("button", { name: "App installieren", exact: true })
      .count(),
    0,
  );
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

  const installed = await contextFor(devices["Pixel 7"], { standalone: true });
  const standalonePage = await installed.newPage();
  await standalonePage.goto(new URL("/login", origin).href);
  await standalonePage
    .getByRole("status")
    .filter({ hasText: "bereits als App" })
    .waitFor();
  assert.equal(
    await standalonePage
      .getByRole("button", { name: "App installieren", exact: true })
      .count(),
    0,
  );
  await installed.close();
  console.log("PASS: standalone mode suppresses repeat installation");

  const settings = await contextFor(devices["Pixel 7"], {
    authenticated: true,
  });
  const settingsPage = await settings.newPage();
  await settingsPage.goto(new URL("/settings", origin).href);
  await settingsPage
    .getByRole("button", { name: "App installieren", exact: true })
    .click();
  await settingsPage
    .getByRole("heading", { name: "Gelabber als App" })
    .waitFor();
  assert.equal(
    await settingsPage.evaluate(
      () => document.documentElement.scrollWidth > window.innerWidth,
    ),
    false,
  );
  await settings.close();
  console.log("PASS: install panel reachable from authenticated settings");
} finally {
  await browser.close();
}
