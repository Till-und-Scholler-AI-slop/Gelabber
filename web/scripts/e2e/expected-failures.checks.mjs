import { test } from "node:test";
import assert from "node:assert/strict";
import { settle } from "./expected-failures.mjs";

const expected = { "known check": "GEL-1" };

test("a listed check that fails and others that pass make a passing run", () => {
  assert.deepEqual(
    settle(expected, {
      passed: ["other check"],
      failed: [{ name: "known check", message: "still broken" }],
    }),
    [],
  );
});

test("an unlisted failure fails the run", () => {
  assert.deepEqual(
    settle(expected, {
      passed: [],
      failed: [
        { name: "known check", message: "still broken" },
        { name: "other check", message: "new break" },
      ],
    }),
    ["other check: new break"],
  );
});

test("a listed check that passes fails the run until its entry is removed", () => {
  assert.deepEqual(settle(expected, { passed: ["known check"], failed: [] }), [
    "known check passes: remove it from the expected failures (GEL-1)",
  ]);
});
