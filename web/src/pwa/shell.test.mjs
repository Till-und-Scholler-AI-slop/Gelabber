// Pins what index.html says about the manifest. The browser smoke
// (scripts/smoke-pwa.mjs) puts a cookie gate in front and checks that Chromium
// then still reads the manifest, but it does not run in CI.
/* global URL */
import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";

const html = readFileSync(new URL("../../index.html", import.meta.url), "utf8");

describe("app shell document", () => {
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
