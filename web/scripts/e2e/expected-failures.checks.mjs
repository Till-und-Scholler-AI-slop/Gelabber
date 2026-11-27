import { test } from "node:test";
import assert from "node:assert/strict";
import { settle } from "./expected-failures.mjs";

const expected = {
  "known check": { issue: "GEL-1", message: /still broken/ },
};

test("a listed check that fails as expected and others that pass make a passing run", () => {
  assert.deepEqual(
    settle(expected, {
      passed: ["other check"],
      failed: [{ name: "known check", message: "it is still broken" }],
    }),
    [],
  );
});

test("an unlisted failure fails the run", () => {
  assert.deepEqual(
    settle(expected, {
      passed: ["third check"],
      failed: [
        { name: "known check", message: "still broken" },
        { name: "other check", message: "new break" },
      ],
    }),
    ["other check: new break"],
  );
});

test("a listed check that fails with another message fails the run", () => {
  assert.deepEqual(
    settle(expected, {
      passed: ["other check"],
      failed: [{ name: "known check", message: "page did not render" }],
    }),
    [
      "known check: page did not render (listed under GEL-1 for /still broken/, failed otherwise)",
    ],
  );
});

test("a listed check that passes fails the run until its entry is removed", () => {
  assert.deepEqual(settle(expected, { passed: ["known check"], failed: [] }), [
    "known check passes: remove it from the expected failures (GEL-1)",
  ]);
});

test("a run in which no check passed fails, even with only expected failures", () => {
  assert.deepEqual(
    settle(expected, {
      passed: [],
      failed: [{ name: "known check", message: "still broken" }],
    }),
    ["no check passed"],
  );
});
