// The parser on inputs that once made its search for a closing mark loop
// forever (GEL-388). A loop never yields, so Vitest's own timeout cannot end
// it: each input is parsed in a child process that a deadline kills, and a
// regression fails by name instead of hanging the run.
/* global URL, process */
import { spawnSync } from "node:child_process";
import { describe, expect, it } from "vitest";

const markdown = new URL("./markdown.ts", import.meta.url).href;
const em = (inside) => ({
  type: "em",
  children: [{ type: "text", text: inside }],
});

describe("markdown", () => {
  it("does not hang on an unclosed backtick or an escape inside italics", () => {
    for (const [input, expected] of [
      ["*a `b*", [em("a `b")]],
      ["*a \\* b*", [em("a * b")]],
      // An unclosed run of backticks stays whole: its second backtick does
      // not open code that hides the closing mark.
      ["*a ``b*` c*", [em("a ``b"), { type: "text", text: "` c*" }]],
    ]) {
      const child = spawnSync(
        process.execPath,
        [
          "--input-type=module",
          "-e",
          `const { parseInline } = await import(${JSON.stringify(markdown)});
          process.stdout.write(JSON.stringify(parseInline(${JSON.stringify(input)})));`,
        ],
        { encoding: "utf8", timeout: 10_000 },
      );
      expect(
        child.signal,
        `parseInline(${JSON.stringify(input)}) still ran after 10 s`,
      ).toBeNull();
      expect(child.status, child.stderr).toBe(0);
      expect(JSON.parse(child.stdout), input).toEqual(expected);
    }
  }, 30_000);
});
