// Pins the rules in web/nginx.conf that decide whether a browser, an
// installed app or the desktop app picks up a new deployment, and the
// security headers it sends. The browser smoke (scripts/smoke-pwa.mjs) checks
// the same against a running nginx, but needs docker and a browser; this runs
// with the unit tests.
/* global URL */
import { createHash } from "node:crypto";
import { readdirSync, readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";

import { fillThemeColours } from "../theme/themeHtml.ts";

const web = new URL("../../", import.meta.url);
const source = readFileSync(new URL("nginx.conf", web), "utf8");

/** Directives per block: `server` and one entry per `location`. */
function parse(text) {
  const blocks = new Map();
  const stack = [];
  let statement = "";
  // Inside quotes `;` and `#` belong to the value: the Content-Security-Policy
  // is a list of directives with `;` between them.
  let quote = null;
  let comment = false;
  for (const char of text) {
    if (comment) {
      if (char === "\n") {
        comment = false;
        statement += char;
      }
      continue;
    }
    if (quote) {
      if (char === quote) quote = null;
      statement += char;
      continue;
    }
    if (char === "#") {
      comment = true;
      continue;
    }
    if (char === '"' || char === "'") quote = char;
    if (char !== ";" && char !== "{" && char !== "}") {
      statement += char;
      continue;
    }
    const words = statement.trim().replace(/\s+/g, " ");
    statement = "";
    if (char === "{") {
      stack.push(words.replace(/^location /, ""));
      if (!blocks.has(stack.at(-1))) blocks.set(stack.at(-1), []);
    } else if (char === "}") stack.pop();
    else if (words) blocks.get(stack.at(-1)).push(words);
  }
  return blocks;
}

const blocks = parse(source);
const location = (name) => blocks.get(name) ?? [];
/** Every `location` in the file, by the name `blocks` keeps it under. */
const locations = [...source.matchAll(/^\s*location\s+([^{]*?)\s*\{/gm)].map(
  ([, name]) => name.replace(/\s+/g, " "),
);
const NO_CACHE = 'add_header Cache-Control "no-cache" always';
const NO_STORE = 'add_header Cache-Control "no-store" always';
const MISSING_IS_404 = "try_files $uri =404";
const SECURITY_HEADERS = [
  'add_header X-Content-Type-Options "nosniff" always',
  'add_header Referrer-Policy "same-origin" always',
  'add_header X-Frame-Options "DENY" always',
  'add_header Permissions-Policy "geolocation=(), payment=(), usb=(), serial=(), hid=(), midi=()" always',
];
/** A location's directives without the security headers every one repeats. */
const ownDirectives = (name) =>
  location(name).filter((line) => !SECURITY_HEADERS.includes(line));
const CSP = "add_header Content-Security-Policy ";

/** The shell's Content-Security-Policy: sources per directive. */
function shellPolicy() {
  const headers = location("= /index.html").filter((line) =>
    line.startsWith(CSP),
  );
  expect(headers).toHaveLength(1);
  const value = headers[0].match(/^add_header \S+ "([^"]*)" always$/)?.[1];
  expect(value, headers[0]).toBeDefined();
  return new Map(
    value
      .split(";")
      .map((directive) => directive.trim().split(" "))
      .filter(([name]) => name)
      .map(([name, ...sources]) => [name, sources]),
  );
}

describe("production nginx config", () => {
  it("lets no browser keep the app shell, including deep links", () => {
    // Not `no-cache`: a restored tab, a discarded tab coming back and a back
    // navigation take a stored copy without asking the server, and the old
    // shell then starts the old client from its immutable files.
    expect(ownDirectives("= /index.html")).toEqual([
      NO_STORE,
      expect.stringMatching(/^add_header Content-Security-Policy /),
    ]);
    // Unknown paths are app routes and end in the shell by internal redirect,
    // which is what brings them under the location above.
    expect(ownDirectives("/")).toEqual(["try_files $uri $uri/ /index.html"]);
  });

  it("serves hashed build output as immutable and never as the app shell", () => {
    expect(location("/assets/")).toContain(MISSING_IS_404);
    // Not `always`: a 404 for a removed chunk must not be cached for a year.
    expect(location("/assets/")).toContain(
      'add_header Cache-Control "public, max-age=31536000, immutable"',
    );
  });

  it("answers 404 for a missing file in every static directory", () => {
    const directories = readdirSync(new URL("public/", web), {
      withFileTypes: true,
    })
      .filter((entry) => entry.isDirectory())
      .map((entry) => `/${entry.name}/`);
    expect(directories).toEqual(
      expect.arrayContaining(["/audio/", "/icons/", "/images/"]),
    );
    for (const directory of directories) {
      expect(location(directory), directory).toContain(MISSING_IS_404);
    }
    // Only `/` may fall back to the shell.
    for (const [name, directives] of blocks) {
      if (name === "/") continue;
      expect(directives.join("\n"), name).not.toContain("/index.html");
    }
  });

  it("has no regex location that would outrank the rules above", () => {
    // nginx asks its regex locations before it settles on a prefix one. A
    // `location ~* \.js$` would take /assets/*.js away from the block that
    // answers 404 for a missing file, and every test above would still pass.
    const regex = [...blocks.keys()].filter((name) => name.startsWith("~"));
    expect(regex).toEqual([]);
  });

  it("keeps manifest, worker and offline page out of the persistent cache", () => {
    for (const file of ["/manifest.webmanifest", "/sw.js", "/offline.html"]) {
      expect(location(`= ${file}`), file).toContain(NO_CACHE);
      expect(location(`= ${file}`), file).toContain(MISSING_IS_404);
    }
    expect(blocks.get("types")).toEqual([
      "application/manifest+json webmanifest",
    ]);
  });

  it("compresses text, also behind the reverse proxy", () => {
    const server = blocks.get("server");
    expect(server).toContain("gzip on");
    // Requests arriving through Caddy carry `Via`; without this nginx would
    // send them uncompressed.
    expect(server).toContain("gzip_proxied any");
    expect(server).toContain("gzip_vary on");
    const types = server.find((line) => line.startsWith("gzip_types "));
    expect(types.split(" ")).toEqual(
      expect.arrayContaining([
        "text/css",
        "application/javascript",
        "application/json",
        "application/manifest+json",
      ]),
    );
    expect(types).not.toContain("wasm");
  });

  it("sends the security headers from every location", () => {
    expect(locations).toEqual(
      expect.arrayContaining([
        "= /manifest.webmanifest",
        "= /sw.js",
        "= /offline.html",
        "= /index.html",
        "/assets/",
        "/icons/",
        "/audio/",
        "/images/",
        "/",
      ]),
    );
    for (const name of locations) {
      for (const header of SECURITY_HEADERS) {
        // Once, with this value, and with `always`, so a 404 has them too.
        const [, field] = header.split(" ");
        expect(
          location(name).filter((line) =>
            line.startsWith(`add_header ${field} `),
          ),
          `${name}: ${field}`,
        ).toEqual([header]);
      }
    }
    // Voice, screen sharing, the output device and copying need these; the
    // YouTube embed is given the rest (LinkPreviews.tsx).
    const permissions = [...blocks.values()]
      .flat()
      .filter((line) => line.startsWith("add_header Permissions-Policy "));
    expect(permissions.length).toBeGreaterThan(0);
    for (const line of permissions) {
      for (const feature of [
        "camera",
        "microphone",
        "display-capture",
        "speaker-selection",
        "clipboard",
        "autoplay",
        "fullscreen",
        "picture-in-picture",
        "encrypted-media",
      ]) {
        expect(line, feature).not.toContain(feature);
      }
    }
  });

  it("sends a Content-Security-Policy about scripts with the shell", () => {
    const policy = shellPolicy();
    expect(
      policy.get("script-src").filter((source) => !source.startsWith("'sha")),
    ).toEqual(["'self'", "'wasm-unsafe-eval'"]);
    expect(policy.get("object-src")).toEqual(["'none'"]);
    expect(policy.get("base-uri")).toEqual(["'self'"]);
    expect(policy.get("form-action")).toEqual(["'self'"]);
    expect(policy.get("frame-ancestors")).toEqual(["'none'"]);
    // The YouTube embed in LinkPreviews.tsx is the app's only frame.
    expect(policy.get("frame-src")).toEqual([
      "https://www.youtube-nocookie.com",
    ]);
    const sources = [...policy.values()].flat();
    expect(sources).not.toContain("'unsafe-inline'");
    expect(sources).not.toContain("'unsafe-eval'");
    // Fetch directives would block uploads and attachments on the MinIO
    // origin, link avatars on any https host and the desktop app's native
    // part on ipc: and http://ipc.localhost (deploy/README.md). style-src
    // would block the inline style of the shell and React's style attributes.
    for (const name of [
      "default-src",
      "connect-src",
      "img-src",
      "media-src",
      "style-src",
    ]) {
      expect(policy.has(name), name).toBe(false);
    }
    // Only the shell: offline.html, the one other document, is static and
    // runs inline scripts of its own.
    for (const [name, directives] of blocks) {
      if (name === "= /index.html") continue;
      expect(
        directives.filter((line) => line.startsWith(CSP)),
        name,
      ).toEqual([]);
    }
  });

  it("lets the shell's inline script run, and no other", () => {
    // As built: the theme colours are filled in at build time.
    const html = fillThemeColours(
      readFileSync(new URL("index.html", web), "utf8"),
    );
    const inline = [...html.matchAll(/<script\b([^>]*)>([\s\S]*?)<\/script>/g)]
      .filter(([, attributes]) => !/\ssrc=/.test(attributes))
      .map(([, , body]) => body);
    // The theme bootstrap. Another inline script needs a hash of its own.
    expect(inline).toHaveLength(1);
    expect(
      shellPolicy()
        .get("script-src")
        .filter((source) => source.startsWith("'sha")),
      "after editing the inline script in index.html, update its hash in nginx.conf",
    ).toEqual(
      inline.map(
        (body) =>
          `'sha256-${createHash("sha256").update(body).digest("base64")}'`,
      ),
    );
  });

  it("sets headers only inside locations", () => {
    // nginx drops every inherited add_header in a block that has its own, so
    // one at server level would silently vanish from the locations above.
    expect(
      blocks.get("server").filter((line) => line.startsWith("add_header")),
    ).toEqual([]);
    expect(blocks.get("server")).toContain("absolute_redirect off");
  });
});
