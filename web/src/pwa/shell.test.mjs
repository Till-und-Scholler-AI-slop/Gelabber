// Pins how the app shell and the entry point are wired to the PWA's parts:
// the links in index.html, the icons the manifest names, and the one call
// that starts worker and notification taps. The browser smoke
// (scripts/smoke-pwa.mjs) checks the same in Chromium against a running
// nginx, but is not part of CI; this runs with the unit tests.
/* global URL */
import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";

const web = new URL("../../", import.meta.url);
const read = (path) => readFileSync(new URL(path, web));
const html = read("index.html").toString("utf8");
const manifest = JSON.parse(
  read("public/manifest.webmanifest").toString("utf8"),
);

/** The attributes of every `<link>` in the shell. */
const links = [...html.matchAll(/<link\b([^>]*)>/g)].map(([, attributes]) =>
  Object.fromEntries(
    [...attributes.matchAll(/([\w-]+)="([^"]*)"/g)].map(([, name, value]) => [
      name,
      value,
    ]),
  ),
);
const link = (rel) => links.filter((entry) => entry.rel === rel);

/** Width and height from the header of a PNG under public/. */
function pngSize(path) {
  expect(path, path).toMatch(/^\/[\w./-]+\.png$/);
  const bytes = read(`public${path}`);
  expect(bytes.subarray(1, 4).toString("latin1"), path).toBe("PNG");
  expect(bytes.subarray(12, 16).toString("latin1"), path).toBe("IHDR");
  return `${bytes.readUInt32BE(16)}x${bytes.readUInt32BE(20)}`;
}

describe("app shell", () => {
  it("links the manifest the server ships", () => {
    expect(link("manifest").map((entry) => entry.href)).toEqual([
      "/manifest.webmanifest",
    ]);
    expect(manifest).toMatchObject({
      id: "/",
      start_url: "/",
      scope: "/",
      display: "standalone",
    });
  });

  it("names icons that exist at the size it states", () => {
    const icons = [...link("apple-touch-icon"), ...link("icon")];
    for (const icon of icons) {
      expect(pngSize(icon.href), icon.href).toBe(icon.sizes);
    }
    // iOS takes the home screen icon from here, not from the manifest.
    expect(link("apple-touch-icon").map((icon) => icon.sizes)).toEqual([
      "180x180",
    ]);
    expect(link("icon").length).toBeGreaterThan(0);
  });

  it("loads the entry point that starts worker and notification taps", () => {
    expect(html).toContain('<script type="module" src="/src/main.tsx">');
    const entry = read("src/main.tsx").toString("utf8");
    // The whole call: the desktop app keeps out of the worker, taps reach
    // the signed-in account, and the router changes route without a reload.
    expect(entry).toContain(
      [
        "startPwa({",
        "  desktop,",
        "  user: () => useSession.getState().user?.id,",
        "  navigate: (to) => router.navigate(to),",
        "});",
      ].join("\n"),
    );
    expect(entry.match(/startPwa\(/g)).toHaveLength(1);
    expect(entry).toMatch(/^const desktop = isDesktopApp\(\);$/m);
  });
});

describe("manifest icons", () => {
  it("exist as PNG files of the stated size", () => {
    expect(manifest.icons.length).toBeGreaterThanOrEqual(3);
    for (const icon of manifest.icons) {
      expect(icon.type, icon.src).toBe("image/png");
      expect(pngSize(icon.src), icon.src).toBe(icon.sizes);
    }
  });

  it("cover both launcher sizes and a maskable one", () => {
    const plain = manifest.icons.filter((icon) => icon.purpose === "any");
    expect(plain.map((icon) => icon.sizes)).toEqual(
      expect.arrayContaining(["192x192", "512x512"]),
    );
    expect(
      manifest.icons.some(
        (icon) => icon.purpose === "maskable" && icon.sizes === "512x512",
      ),
    ).toBe(true);
  });
});

describe("app shell document behind an access proxy", () => {
  it("asks for the manifest with the cookies of the site", () => {
    // A browser fetches a manifest without credentials unless the link says
    // otherwise, even on the same origin. Behind an access proxy that lets
    // nothing through without its cookie (Authelia, oauth2-proxy, Cloudflare
    // Access) the request ends at the proxy's login, the browser has no
    // manifest and never offers installation.
    const links = html.match(/<link\b[^>]*\brel="manifest"[^>]*>/g) ?? [];
    expect(links).toHaveLength(1);
    expect(links[0]).toMatch(/\shref="\/manifest\.webmanifest"/);
    expect(links[0]).toMatch(/\scrossorigin="use-credentials"/);
  });
});
