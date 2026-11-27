/* global console */
import { test } from "node:test";
import assert from "node:assert/strict";
import { ciFrom, missing } from "./prerequisite.mjs";

test("outside CI a missing prerequisite is skipped and says so", (t) => {
  const log = t.mock.method(console, "log", () => {});
  missing("a tool", { ci: false });
  assert.deepEqual(log.mock.calls[0].arguments, [
    "Skipped: a tool is missing on this machine (with CI set, this fails).",
  ]);
});

test("in CI a missing prerequisite fails", () => {
  assert.throws(() => missing("a tool", { ci: true }), {
    message:
      "a tool is missing; CI must provide it (GEL-375). (CI is set; unset it to skip here)",
  });
});

test("CI counts unless it is unset, empty, 0 or false", () => {
  for (const [value, ci] of [
    ["true", true],
    ["1", true],
    ["yes", true],
    ["", false],
    ["0", false],
    ["false", false],
    ["False", false],
    [undefined, false],
  ])
    assert.equal(ciFrom(value), ci, `CI=${value}`);
});
