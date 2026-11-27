import { test } from "node:test";
import assert from "node:assert/strict";
import { runWithin } from "./run-within.mjs";
/* global process, setTimeout */

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
  const ok = await runWithin(["-e", "console.log(process.env.SAY)"], {
    expired: never,
    env: { SAY: "fine" },
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

const alive = (pid) => {
  try {
    process.kill(pid, 0);
    return true;
  } catch {
    return false;
  }
};

test(
  "a child killed when its time runs out takes the processes it started with it",
  { skip: process.platform === "win32" && "process groups: Linux and macOS" },
  async () => {
    // The grandchild stands for a browser the check started; the child
    // writes its pid first, and its time runs out on that.
    const result = await runWithin(
      [
        "-e",
        `const { spawn } = require("node:child_process");
         const browser = spawn(process.execPath, ["-e", "setInterval(() => {}, 1000)"], { stdio: "ignore" });
         console.log("browser " + browser.pid);
         setInterval(() => {}, 1000);`,
      ],
      { expired: ({ started }) => started },
    );
    assert.equal(result.killed, true);
    const pid = Number(/browser (\d+)/.exec(String(result.output))?.[1]);
    assert.ok(pid > 0, String(result.output));
    // SIGKILL to the group is sent; wait until the kernel has taken it.
    for (let i = 0; i < 500 && alive(pid); i++)
      await new Promise((resolve) => setTimeout(resolve, 10));
    assert.equal(alive(pid), false, `pid ${pid} outlived its check`);
  },
);
