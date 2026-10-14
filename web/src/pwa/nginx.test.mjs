// Pins the rules in web/nginx.conf that decide whether a browser, an
// installed app or the desktop app picks up a new deployment. The browser
// smoke (scripts/smoke-pwa.mjs) checks the same against a running nginx, but
// needs docker and a browser; this runs with the unit tests.
/* global URL */
import { readdirSync, readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";

const web = new URL("../../", import.meta.url);
const source = readFileSync(new URL("nginx.conf", web), "utf8");

/** Directives per block: `server` and one entry per `location`. */
function parse(text) {
  const blocks = new Map();
  const stack = [];
  let statement = "";
  for (const char of text.replace(/#.*$/gm, "")) {
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
const NO_CACHE = 'add_header Cache-Control "no-cache" always';
const NO_STORE = 'add_header Cache-Control "no-store" always';
const MISSING_IS_404 = "try_files $uri =404";

describe("production nginx config", () => {
  it("lets no browser keep the app shell, including deep links", () => {
    // Not `no-cache`: a restored tab, a discarded tab coming back and a back
    // navigation take a stored copy without asking the server, and the old
    // shell then starts the old client from its immutable files.
    expect(location("= /index.html")).toEqual([NO_STORE]);
    // Unknown paths are app routes and end in the shell by internal redirect,
    // which is what brings them under the location above.
    expect(location("/")).toEqual(["try_files $uri $uri/ /index.html"]);
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

  it("sets headers only inside locations", () => {
    // nginx drops every inherited add_header in a block that has its own, so
    // one at server level would silently vanish from the locations above.
    expect(
      blocks.get("server").filter((line) => line.startsWith("add_header")),
    ).toEqual([]);
    expect(blocks.get("server")).toContain("absolute_redirect off");
  });
});
