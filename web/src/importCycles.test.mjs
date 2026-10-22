// Keeps the app's modules free of import cycles (#291). Inside a cycle the
// order in which modules run depends on which one is imported first, so a
// test or an entry point can read a binding before its module ran. Type-only
// imports are erased by the compiler and do not count.
/* global URL */
import { readdirSync, readFileSync, statSync } from "node:fs";
import { dirname, join, relative, sep } from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";

const src = fileURLToPath(new URL(".", import.meta.url));
const EXTENSIONS = [".ts", ".tsx", ".js", ".mjs"];

/** The source with comments blanked; string literals stay as they are. */
const withoutComments = (text) =>
  text.replace(
    /("(?:\\.|[^"\\\n])*"|'(?:\\.|[^'\\\n])*'|`(?:\\.|[^`\\])*`)|\/\*[\s\S]*?\*\/|\/\/[^\n]*/g,
    (match, literal) => literal ?? " ",
  );

/**
 * Specifiers of the static imports and re-exports that still exist at
 * runtime. `import type` and `export type` are skipped; `import { type X }`
 * counts, since the module may still be loaded for it.
 */
function valueImports(text) {
  const statement =
    /^[ \t]*(?:(?:import|export)\b([^;"'`]*?)\bfrom|import)\s*(["'])([^"'\n]+)\2/gm;
  return [...withoutComments(text).matchAll(statement)]
    .filter(([, clause]) => !/^\s*type\s+(?!from\b)\S/.test(clause ?? ""))
    .map(([, , , specifier]) => specifier);
}

/** The file a relative specifier names, the way the bundler finds it. */
function resolveImport(importer, specifier) {
  const base = join(dirname(importer), specifier.replace(/[?#].*$/, ""));
  return [
    base,
    ...EXTENSIONS.map((extension) => base + extension),
    ...EXTENSIONS.map((extension) => join(base, `index${extension}`)),
  ].find((path) => statSync(path, { throwIfNoEntry: false })?.isFile());
}

const name = (path) => relative(src, path).split(sep).join("/");

/** Value edges between the modules under src, by module path. */
function importGraph() {
  const modules = readdirSync(src, { recursive: true, withFileTypes: true })
    .filter(
      (entry) =>
        entry.isFile() &&
        EXTENSIONS.some((extension) => entry.name.endsWith(extension)) &&
        !/\.test\.|\.d\.ts$/.test(entry.name),
    )
    .map((entry) => join(entry.parentPath, entry.name));
  const graph = new Map(modules.map((module) => [module, new Set()]));
  const unresolved = [];
  for (const module of modules) {
    for (const specifier of valueImports(readFileSync(module, "utf8"))) {
      if (!/^\.\.?\//.test(specifier)) continue;
      const target = resolveImport(module, specifier);
      if (!target) unresolved.push(`${name(module)}: ${specifier}`);
      else if (graph.has(target)) graph.get(module).add(target);
    }
  }
  return { graph, unresolved };
}

/**
 * Tarjan's strongly connected components with more than one module, or with
 * a module that imports itself. Each is listed by the edges inside it, which
 * are the imports one of which has to go.
 */
function cycles(graph) {
  let next = 0;
  const index = new Map();
  const low = new Map();
  const stack = [];
  const onStack = new Set();
  const found = [];
  const visit = (module) => {
    index.set(module, next);
    low.set(module, next);
    next += 1;
    stack.push(module);
    onStack.add(module);
    for (const target of graph.get(module)) {
      if (!index.has(target)) {
        visit(target);
        low.set(module, Math.min(low.get(module), low.get(target)));
      } else if (onStack.has(target)) {
        low.set(module, Math.min(low.get(module), index.get(target)));
      }
    }
    if (low.get(module) !== index.get(module)) return;
    const component = new Set();
    let member;
    do {
      member = stack.pop();
      onStack.delete(member);
      component.add(member);
    } while (member !== module);
    if (component.size > 1 || graph.get(module).has(module)) {
      found.push(
        [...component]
          .flatMap((from) =>
            [...graph.get(from)]
              .filter((to) => component.has(to))
              .map((to) => `${name(from)} -> ${name(to)}`),
          )
          .sort(),
      );
    }
  };
  for (const module of graph.keys()) if (!index.has(module)) visit(module);
  return found;
}

describe("import cycles in src", () => {
  const { graph, unresolved } = importGraph();
  const importsOf = (module) =>
    [...(graph.get(join(src, module)) ?? [])].map(name);

  it("reads value imports and re-exports, and skips type-only ones", () => {
    expect(
      valueImports(`
import a from "./a.ts";
import {
  b,
  type C,
} from "./b.ts";
import * as d from "../d.ts";
import "./e.css";
import type { F } from "./f.ts";
import type G from "./g.ts";
export { h, type I } from "./h.ts";
export * from "./j.ts";
export type { K } from "./k.ts";
// import l from "./l.ts";
/* import m from "./m.ts"; */
const n = "/* not a comment";
import o from 'package';
// */
const p = await import("./p.ts");
`),
    ).toEqual([
      "./a.ts",
      "./b.ts",
      "../d.ts",
      "./e.css",
      "./h.ts",
      "./j.ts",
      "package",
    ]);
  });

  it("finds the modules and resolves every relative import", () => {
    expect(unresolved).toEqual([]);
    // Edges of the tree itself, so a broken reader cannot pass vacuously:
    // one after a comment block, and none for a type-only import.
    expect(importsOf("auth/scope.ts")).toContain("auth/store.ts");
    expect(importsOf("voice/sessionState.ts")).toContain("auth/store.ts");
    expect(importsOf("auth/release.ts")).toContain("voice/session.ts");
    expect(importsOf("auth/session.ts")).toContain("auth/store.ts");
    expect(importsOf("auth/session.ts")).not.toContain("auth/types.ts");
  });

  it("has no cycles", () => {
    expect(cycles(graph)).toEqual([]);
  });
});
