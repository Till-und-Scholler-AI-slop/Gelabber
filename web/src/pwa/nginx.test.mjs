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
  let variable = false;
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
    // The braces of `${uri}` belong to the variable, not to a block.
    if (char === "{" && statement.endsWith("$")) variable = true;
    if (variable || (char !== ";" && char !== "{" && char !== "}")) {
      if (char === "}") variable = false;
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

const LANDING_MAP = 'map "" $gelabber_landing';
const SIGNED_OUT_MAP = "map $cookie_gelabber_session $gelabber_signed_out";
const LANG_MAP = "map $http_accept_language $gelabber_public_lang";
const FRONT_MAP =
  'map "$gelabber_landing$gelabber_signed_out$is_args" $gelabber_front';
const publicPagesScript = fileURLToPath(
  new URL("docker-entrypoint.d/50-gelabber-public-pages.sh", web),
);
/** Where the public pages live: the script's output folder. */
const PUBLIC_ROOT = "root /usr/share/nginx/public-pages";
const PAGE_CSP =
  "default-src 'none'; style-src 'self'; img-src 'self'; font-src 'self'; base-uri 'none'; form-action 'none'; frame-ancestors 'none'";

const unquote = (word) => word.replace(/^"(.*)"$/, "$1");

/** A map's value for `key` as nginx picks it: an exact entry, else the first
 *  matching regex, else default. `included` are the lines of the files its
 *  `include` reads. */
function lookup(map, key, included = []) {
  const lines = (blocks.get(map) ?? []).flatMap((line) =>
    line.startsWith("include ") ? included : [line],
  );
  const entries = lines.map((line) => {
    const [, name, value] = line.match(/^("[^"]*"|\S+) (.*)$/);
    return [unquote(name), unquote(value)];
  });
  const exact = entries.find(([name]) => name === key && !name.startsWith("~"));
  if (exact) return exact[1];
  const regex = entries.find(
    ([name]) =>
      name.startsWith("~") &&
      new RegExp(
        name.replace(/^~\*?/, ""),
        name.startsWith("~*") ? "i" : "",
      ).test(key),
  );
  if (regex) return regex[1];
  return entries.find(([name]) => name === "default")?.[1];
}

/** Where `location = /` sends a request, with these maps' inputs. */
function front({ landing, cookie = "", query = "", language = "" }) {
  // What the entrypoint writes while the landing page is on
  // (src/public/entrypoint.test.mjs).
  const on = landing ? ['"" 1'] : [];
  const key =
    lookup(LANDING_MAP, "", on) +
    lookup(SIGNED_OUT_MAP, cookie) +
    (query ? "?" : "");
  const rewrite = location("= /").filter((line) => line.startsWith("rewrite "));
  expect(rewrite).toEqual(["rewrite ^ $gelabber_front last"]);
  return lookup(FRONT_MAP, key).replace(
    "$gelabber_public_lang",
    lookup(LANG_MAP, language),
  );
}

/** The file a public-page location serves for `uri`, given the files the
 *  entrypoint wrote, or the URI its try_files falls back to. */
function served(name, uri, files) {
  const tries = location(name).filter((line) => line.startsWith("try_files "));
  expect(tries, name).toHaveLength(1);
  const [, ...candidates] = tries[0].split(" ");
  const fallback = candidates.pop();
  for (const candidate of candidates) {
    const path = candidate.replaceAll("${uri}", uri).replaceAll("$uri", uri);
    if (files.includes(path.replace(/^\//, ""))) return path;
  }
  return fallback;
}

/** Sources per directive of a Content-Security-Policy header line. */
function policyOf(line) {
  const value = line.match(/^add_header \S+ "([^"]*)" always$/)?.[1];
  expect(value, line).toBeDefined();
  return new Map(
    value
      .split(";")
      .map((directive) => directive.trim().split(" "))
      .filter(([name]) => name)
      .map(([name, ...sources]) => [name, sources]),
  );
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

  it("gives the unhashed static files a cache lifetime", () => {
    // Their names stay across releases, so not immutable. The RNNoise worklet
    // and wasm must match the build that loads them: asked again each time.
    for (const directory of ["/icons/", "/images/"])
      expect(location(directory), directory).toContain(
        'add_header Cache-Control "public, max-age=86400"',
      );
    expect(location("/audio/")).toContain(
      'add_header Cache-Control "no-cache"',
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
    // Only `/` may fall back to the shell, besides `location = /` (through the
    // map) and the public pages, whose unpublished URLs are app routes.
    const toShell = [
      "/",
      FRONT_MAP,
      "^~ /de/",
      "^~ /en/",
      "^~ /de/feedback",
      "^~ /en/feedback",
    ];
    for (const [name, directives] of blocks) {
      if (toShell.includes(name)) continue;
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
        "= /",
        "^~ /de/",
        "^~ /en/",
        "^~ /de/feedback",
        "^~ /en/feedback",
        "= /public-pages.json",
        "= /robots.txt",
        "= /sitemap.xml",
        "^~ /public-pages/",
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
    // runs inline scripts of its own. The public pages have a policy of their
    // own (below).
    for (const [name, directives] of blocks) {
      if (
        [
          "= /index.html",
          "^~ /de/",
          "^~ /en/",
          "^~ /de/feedback",
          "^~ /en/feedback",
        ].includes(name)
      )
        continue;
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
    // the gateway and media WebSockets ('self'), the Tauri app's IPC.
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

describe("public pages in the nginx config (#522)", () => {
  it("keeps / the app shell unless the landing page is on", () => {
    // A self-hosted instance by default: no landing map, so / goes to the
    // shell and from there to sign-in, as before.
    for (const cookie of ["", "token"])
      for (const query of ["", "?next=/s/x"])
        for (const language of ["", "de-DE", "en"])
          expect(
            front({ landing: false, cookie, query, language }),
            `${cookie} ${query} ${language}`,
          ).toBe("/index.html");
  });

  it("serves the landing page at / only to a signed-out visitor without a query string", () => {
    expect(front({ landing: true })).toBe("/en/");
    // Signed in, or a session that has expired: the app decides.
    expect(front({ landing: true, cookie: "token" })).toBe("/index.html");
    // A link into the app with parameters, such as an OAuth return.
    expect(front({ landing: true, query: "?code=1" })).toBe("/index.html");
    expect(front({ landing: true, cookie: "token", query: "?a" })).toBe(
      "/index.html",
    );
  });

  it("picks German only when the browser lists German first", () => {
    const lang = (language) => front({ landing: true, language });
    expect(lang("de-DE,de;q=0.9,en;q=0.8")).toBe("/de/");
    expect(lang("de")).toBe("/de/");
    expect(lang(" DE-AT")).toBe("/de/");
    expect(lang("en-GB,de;q=0.9")).toBe("/en/");
    expect(lang("fr-FR, de")).toBe("/en/");
    expect(lang("")).toBe("/en/");
    expect(lang("*")).toBe("/en/");
  });

  it("serves the generated pages under /de/ and /en/, and the shell for the rest", () => {
    const written = [
      "de/index.html",
      "de/impressum.html",
      "en/index.html",
      "en/imprint.html",
    ];
    for (const lang of ["de", "en"]) {
      const name = `^~ /${lang}/`;
      expect(location(name), name).toContain(PUBLIC_ROOT);
      expect(served(name, `/${lang}/`, written)).toBe(`/${lang}/index.html`);
      expect(served(name, `/${lang}/nope`, written)).toBe("/index.html");
      expect(served(name, `/${lang}/`, []), "landing off").toBe("/index.html");
    }
    expect(served("^~ /de/", "/de/impressum", written)).toBe(
      "/de/impressum.html",
    );
    expect(served("^~ /en/", "/en/imprint", written)).toBe("/en/imprint.html");
    // Not published: the app's not-found page.
    expect(served("^~ /en/", "/en/privacy", written)).toBe("/index.html");
    // Prefix locations that no regex location can take over (above), and the
    // output folder the entrypoint writes.
    expect(readFileSync(publicPagesScript, "utf8")).toContain(
      "OUT=${GELABBER_PUBLIC_OUT:-/usr/share/nginx/public-pages}",
    );
  });

  it("sends the public pages with no-cache, Vary and a policy that allows no script", () => {
    for (const name of ["^~ /de/", "^~ /en/"]) {
      // The security headers are checked for every location above.
      expect(ownDirectives(name), name).toEqual([
        PUBLIC_ROOT,
        NO_CACHE,
        'add_header Vary "Accept-Language, Cookie" always',
        `add_header Content-Security-Policy "${PAGE_CSP}" always`,
        "try_files ${uri}index.html $uri.html /index.html",
      ]);
      const policy = policyOf(
        location(name).find((line) => line.startsWith(CSP)),
      );
      // Without script-src, default-src 'none' covers scripts.
      expect(policy.has("script-src"), name).toBe(false);
      expect(policy.get("default-src")).toEqual(["'none'"]);
      for (const [directive, sources] of policy)
        expect(sources, directive).not.toEqual(
          expect.arrayContaining(["'unsafe-inline'"]),
        );
    }
  });

  it("lets only the feedback form post, and only to this origin (#542)", () => {
    const formAction = (name) =>
      policyOf(location(name).find((line) => line.startsWith(CSP))).get(
        "form-action",
      );
    for (const lang of ["de", "en"]) {
      const name = `^~ /${lang}/feedback`;
      // The block of /de/ and /en/, with form-action 'self' as the one change.
      expect(ownDirectives(name), name).toEqual(
        ownDirectives(`^~ /${lang}/`).map((line) =>
          line.replace("form-action 'none'", "form-action 'self'"),
        ),
      );
      expect(formAction(name), name).toEqual(["'self'"]);
      expect(formAction(`^~ /${lang}/`), lang).toEqual(["'none'"]);
      const written = [
        `${lang}/feedback.html`,
        `${lang}/feedback/${lang === "de" ? "gesendet" : "sent"}.html`,
      ];
      expect(served(name, `/${lang}/feedback`, written)).toBe(
        `/${lang}/feedback.html`,
      );
      expect(
        served(
          name,
          `/${lang}/feedback/${lang === "de" ? "gesendet" : "sent"}`,
          written,
        ),
      ).toBe(`/${written[1]}`);
      // Not published: the app's not-found page, as for the other pages.
      expect(served(name, `/${lang}/feedback`, [])).toBe("/index.html");
    }
    // Every other public page and the shell keep their policies.
    const formActions = [...blocks]
      .filter(([name]) => !name.endsWith("/feedback"))
      .flatMap(([, directives]) => directives)
      .filter((line) => line.startsWith(CSP))
      .map((line) => policyOf(line).get("form-action"));
    expect(formActions).toEqual([["'self'"], ["'none'"], ["'none'"]]);
  });

  it("answers 404 for public-pages.json, robots.txt and sitemap.xml until they are written", () => {
    for (const file of ["public-pages.json", "robots.txt", "sitemap.xml"]) {
      const name = `= /${file}`;
      expect(ownDirectives(name), name).toEqual([
        PUBLIC_ROOT,
        NO_CACHE,
        MISSING_IS_404,
      ]);
      expect(served(name, `/${file}`, [])).toBe("=404");
      expect(served(name, `/${file}`, [file])).toBe(`/${file}`);
    }
  });

  it("keeps the build's unfilled templates private", () => {
    // The entrypoint reads them from the web root; their placeholders must
    // never be served.
    expect(readFileSync(publicPagesScript, "utf8")).toContain(
      "TEMPLATES=${GELABBER_PUBLIC_TEMPLATES:-/usr/share/nginx/html/public-pages/templates}",
    );
    expect(ownDirectives("^~ /public-pages/")).toEqual(["return 404"]);
  });

  it("leaves the shell's policy as it was", () => {
    expect(
      location("= /index.html").filter((line) => line.startsWith(CSP)),
    ).toEqual([
      `add_header Content-Security-Policy "script-src 'self' 'wasm-unsafe-eval' 'sha256-Ymhy9okZfEMbRFjgkub854NfFwTSwdNReGSoXDKUA3M='; object-src 'none'; base-uri 'self'; form-action 'self'; frame-src https://www.youtube-nocookie.com; frame-ancestors 'none'$gelabber_fetch_directives" always`,
    ]);
  });

  it("reads the landing map from a folder the storage map's include does not", () => {
    const include = (map) =>
      blocks
        .get(map)
        .find((line) => line.startsWith("include "))
        .replace(/^include /, "");
    // An nginx glob: `*` never crosses a `/`.
    const matches = (glob, path) =>
      new RegExp(
        `^${glob.replace(/[.]/g, "\\.").replaceAll("*", "[^/]*")}$`,
      ).test(path);
    const landingMap = readFileSync(publicPagesScript, "utf8").match(
      /MAP=\$\{GELABBER_LANDING_MAP:-([^}]+)\}/,
    )?.[1];
    expect(landingMap).toBe("/etc/nginx/gelabber/public/landing.map");
    expect(matches(include(LANDING_MAP), landingMap)).toBe(true);
    expect(matches(include(STORAGE_MAP), landingMap)).toBe(false);
    expect(
      matches(include(LANDING_MAP), "/etc/nginx/gelabber/storage-origin.map"),
    ).toBe(false);
    expect(blocks.get(LANDING_MAP)).toEqual([
      "default 0",
      "include /etc/nginx/gelabber/public/*.map",
    ]);
  });
});
