// Pins the rules in web/nginx.conf that decide whether a browser, an
// installed app or the desktop app picks up a new deployment, and the
// security headers it sends. The browser smoke (scripts/smoke-pwa.mjs) checks
// the same against a running nginx, but needs docker and a browser; this runs
// with the unit tests.
/* global URL, process */
import { execFileSync } from "node:child_process";
import { createHash } from "node:crypto";
import {
  existsSync,
  mkdtempSync,
  readdirSync,
  readFileSync,
  rmSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
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

const STORAGE_MAP = 'map "" $gelabber_storage_origin';
const FETCH_MAP = "map $gelabber_storage_origin $gelabber_fetch_directives";
const entrypoint = fileURLToPath(
  new URL("docker-entrypoint.d/40-gelabber-storage-origin.sh", web),
);

/** A map's value for `key`, as nginx picks it: the key's entry or default. */
function mapped(map, key) {
  const entries = new Map(
    (blocks.get(map) ?? [])
      .map((line) => line.match(/^("[^"]*"|\S+) "([^"]*)"$/))
      .filter(Boolean)
      .map(([, name, value]) => [name, value]),
  );
  return entries.get(`"${key}"`) ?? entries.get("default");
}

/** The shell's Content-Security-Policy: sources per directive, with the
 *  MinIO origin the entrypoint hands over (none: the container has none). */
function shellPolicy(storage = "") {
  const headers = location("= /index.html").filter((line) =>
    line.startsWith(CSP),
  );
  expect(headers).toHaveLength(1);
  const value = headers[0]
    .match(/^add_header \S+ "([^"]*)" always$/)?.[1]
    ?.replace(
      "$gelabber_fetch_directives",
      mapped(FETCH_MAP, storage).replaceAll(
        "$gelabber_storage_origin",
        storage,
      ),
    );
  expect(value, headers[0]).toBeDefined();
  expect(value).not.toContain("$");
  const directives = value
    .split(";")
    .map((directive) => directive.trim().split(" "))
    .filter(([name]) => name);
  // A browser takes a repeated directive only the first time.
  expect(new Set(directives.map(([name]) => name)).size).toBe(
    directives.length,
  );
  return new Map(directives.map(([name, ...sources]) => [name, sources]));
}

/** What the entrypoint script writes for this MINIO_PUBLIC_ENDPOINT, and
 *  what it logs. `null`: no file, so no fetch directives. */
function storageFile(endpoint, delivery) {
  const dir = mkdtempSync(join(tmpdir(), "gelabber-storage-"));
  const file = join(dir, "gelabber", "storage-origin.map");
  try {
    const env = { PATH: process.env.PATH, GELABBER_STORAGE_MAP: file };
    if (endpoint !== undefined) env.MINIO_PUBLIC_ENDPOINT = endpoint;
    if (delivery !== undefined) env.ATTACHMENT_DELIVERY = delivery;
    const log = execFileSync("sh", [entrypoint], { env, encoding: "utf8" });
    return {
      written: existsSync(file) ? readFileSync(file, "utf8") : null,
      log,
    };
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
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
    // Without the MinIO origin fetch directives would block uploads and
    // attachments: a container from an older Compose file has none.
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

  it("adds fetch directives once the MinIO origin is known", () => {
    const storage = "https://minio.example.com";
    const policy = shellPolicy(storage);
    // The directives without the origin stay as they are.
    for (const [name, sources] of shellPolicy())
      expect(policy.get(name), name).toEqual(sources);
    expect(policy.get("default-src")).toEqual(["'self'"]);
    // Presigned uploads, text attachments (fetched through the redirect),
    // the gateway and media WebSockets ('self'), the desktop app's IPC.
    expect(policy.get("connect-src")).toEqual([
      "'self'",
      storage,
      "ipc:",
      "http://ipc.localhost",
    ]);
    // Attachments through the redirect, the favicon badge (data:), upload
    // previews (blob:), link avatars on any https host.
    expect(policy.get("img-src")).toEqual([
      "'self'",
      "data:",
      "blob:",
      "https:",
      storage,
    ]);
    // Video and audio attachments, call sounds (data:), microphone test
    // clips (blob:). No other host.
    expect(policy.get("media-src")).toEqual([
      "'self'",
      "data:",
      "blob:",
      storage,
    ]);
    // The shell's inline <style>; without style-src, default-src applies.
    expect(policy.get("style-src")).toEqual(["'self'", "'unsafe-inline'"]);
    for (const [name, sources] of policy) {
      if (name !== "style-src")
        expect(sources, name).not.toContain("'unsafe-inline'");
      expect(sources, name).not.toContain("*");
    }
  });

  it("names no storage host when attachments go through the API (#348)", () => {
    const policy = shellPolicy("'self'");
    for (const [name, sources] of shellPolicy())
      expect(policy.get(name), name).toEqual(sources);
    expect(policy.get("default-src")).toEqual(["'self'"]);
    expect(policy.get("connect-src")).toEqual([
      "'self'",
      "ipc:",
      "http://ipc.localhost",
    ]);
    expect(policy.get("img-src")).toEqual([
      "'self'",
      "data:",
      "blob:",
      "https:",
    ]);
    expect(policy.get("media-src")).toEqual(["'self'", "data:", "blob:"]);
    expect(policy.get("style-src")).toEqual(["'self'", "'unsafe-inline'"]);
  });

  it.skipIf(process.platform === "win32")(
    "hands nginx 'self' with ATTACHMENT_DELIVERY=proxy, whatever the endpoint",
    () => {
      for (const endpoint of [
        undefined,
        "https://sscholler.de:9000",
        "http://localhost:9000",
      ]) {
        for (const delivery of ["proxy", "PROXY", " proxy "]) {
          const { written, log } = storageFile(endpoint, delivery);
          expect(written, `${endpoint} ${delivery}`).toBe(`"" "'self'";\n`);
          expect(log).toContain("go through the API");
        }
      }
      // Any other value is redirect delivery: MinIO's origin as before.
      for (const delivery of ["", "redirect", "stream"])
        expect(
          storageFile("https://minio.example.com", delivery).written,
          delivery,
        ).toBe('"" "https://minio.example.com";\n');
    },
  );

  it("takes the MinIO origin from the entrypoint's file only", () => {
    // Mounted on its own, the file matches nothing and the origin is empty.
    expect(blocks.get(STORAGE_MAP)).toEqual([
      'default ""',
      "include /etc/nginx/gelabber/*.map",
    ]);
    expect(mapped(FETCH_MAP, "")).toBe("");
    expect(readFileSync(entrypoint, "utf8")).toContain(
      "/etc/nginx/gelabber/storage-origin.map}",
    );
  });

  it.skipIf(process.platform === "win32")(
    "hands nginx the origin of MINIO_PUBLIC_ENDPOINT, or nothing",
    () => {
      const origin = (endpoint) => storageFile(endpoint).written;
      expect(origin("https://minio.example.com")).toBe(
        '"" "https://minio.example.com";\n',
      );
      // Path and case do not matter; the port does.
      expect(origin("HTTPS://Minio.Example.com/gelabber/")).toBe(
        '"" "https://minio.example.com";\n',
      );
      expect(origin("http://localhost:9000")).toBe(
        '"" "http://localhost:9000";\n',
      );
      expect(origin("http://192.168.1.20:9000")).toBe(
        '"" "http://192.168.1.20:9000";\n',
      );
      // Nothing a policy could name, or could be broken out of: no file, so
      // the policy keeps its fetch directives out instead of blocking MinIO.
      for (const endpoint of [
        undefined,
        "",
        "minio.example.com",
        "ftp://minio.example.com",
        "http://[::1]:9000",
        "http://minio_1:9000",
        "https://user:pass@minio.example.com",
        'https://minio.example.com"; default "x',
        "https://minio.example.com; script-src *",
        "https://minio.example.com:9000:1",
      ]) {
        const { written, log } = storageFile(endpoint);
        expect(written, String(endpoint)).toBeNull();
        expect(log, String(endpoint)).toContain("no fetch directives");
      }
    },
  );

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
