import { test } from "node:test";
import assert from "node:assert/strict";
import { runWithin } from "./run-within.mjs";

const never = () => new Promise(() => {});

test("a child that never exits is killed when its time runs out, and keeps what it wrote", async () => {
  const result = await runWithin(
    ["-e", 'console.log("started"); setInterval(() => {}, 1000);'],
    // Its time runs out once it has written something: no clock involved.
    { expired: ({ started }) => started },
  );
  assert.equal(result.killed, true);
  assert.notEqual(result.code, 0);
  assert.match(String(result.output), /started/);
});

test("a child that ends in time is not killed, and its exit code and output come back", async () => {
  const ok = await runWithin(["-e", 'console.log("fine")'], {
    expired: never,
  });
  assert.deepEqual(
    { killed: ok.killed, code: ok.code, output: String(ok.output) },
    { killed: false, code: 0, output: "fine\n" },
  );
  const failed = await runWithin(
    ["-e", 'console.error("broken"); process.exit(3)'],
    { expired: never },
  );
  assert.deepEqual(
    {
      killed: failed.killed,
      code: failed.code,
      output: String(failed.output),
    },
    { killed: false, code: 3, output: "broken\n" },
  );
});
