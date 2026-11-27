// Every script under web/scripts is named somewhere that runs or documents
// it: web/scripts/README.md, package.json, checks.mjs, a workflow, or a path in
// a script that is (an import, `new URL("./x.py", import.meta.url)`, a
// `"scripts/…"` argument). A new script nobody names fails here by name.
import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync, readdirSync } from "node:fs";
import { dirname, join, posix } from "node:path";

const web = new URL("..", import.meta.url).pathname;
const workflows = join(web, "../.github/workflows");

/** Scripts (relative to web/scripts) that no source names, nor a path in a
 * named script. `read(file)` returns a script's text. */
export function unaccounted(scripts, sources, read) {
  const named = (file) => {
    const stem = file.replace(/\.(mjs|py|sh)$/, "");
    const escape = (text) => text.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
    const word = new RegExp(
      `(^|[^\\w-])${escape(stem)}(\\.(mjs|py|sh))?(?![\\w.-])`,
    );
    return sources.some(
      (text) =>
        word.test(text) ||
        // A folder (`parity/`) or a pattern (`audio/build-*.sh`) names its files.
        [...text.matchAll(/`([\w./*-]+)`/g)].some(([, name]) =>
          name.endsWith("/")
            ? file.startsWith(name)
            : name.includes("*") &&
              new RegExp(`^${name.split("*").map(escape).join("[^/]*")}`).test(
                file,
              ),
        ),
    );
  };
  const accounted = new Set(scripts.filter(named));
  for (let grew = true; grew;) {
    grew = false;
    for (const file of [...accounted])
      for (const [, path] of read(file).matchAll(
        /["']((?:\.{1,2}\/|scripts\/)[^"']+\.(?:mjs|py|sh))["']/g,
      )) {
        const target = path.startsWith("scripts/")
          ? path.slice("scripts/".length)
          : posix.normalize(posix.join(dirname(file), path));
        if (scripts.includes(target) && !accounted.has(target)) {
          accounted.add(target);
          grew = true;
        }
      }
  }
  return scripts.filter((file) => !accounted.has(file));
}

const scripts = readdirSync(join(web, "scripts"), { recursive: true })
  .map(String)
  .filter(
    (file) => /\.(mjs|py|sh)$/.test(file) && !file.includes("node_modules"),
  );
const sources = [
  join(web, "scripts/README.md"),
  join(web, "package.json"),
  join(web, "scripts/checks.mjs"),
  ...readdirSync(workflows)
    .filter((file) => file.endsWith(".yml"))
    .map((file) => join(workflows, file)),
].map((path) => readFileSync(path, "utf8"));
const read = (file) => readFileSync(join(web, "scripts", file), "utf8");

test("every script under web/scripts is named where it runs or is documented", () => {
  assert.deepEqual(unaccounted(scripts, sources, read), []);
});

test("a script nobody names is reported by its path", () => {
  const extra = "zz-unaccounted.mjs";
  assert.deepEqual(
    unaccounted([...scripts, extra], sources, (file) =>
      file === extra ? "" : read(file),
    ),
    [extra],
  );
});
