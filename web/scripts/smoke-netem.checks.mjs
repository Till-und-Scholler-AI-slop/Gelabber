import { test } from "node:test";
import assert from "node:assert/strict";
import { netemDrops, udpFilters, withUdpLoss } from "./smoke-netem.mjs";

const counters = (drops) =>
  JSON.stringify([
    { kind: "prio", handle: "7a11:", drops: 999 },
    { kind: "netem", handle: "7a12:", parent: "7a11:3", drops },
  ]);
test("actual ICE ports only match UDP; TCP and other UDP ports default unshaped", () => {
  const filters = udpFilters([10000, 10031, 10000]);
  assert.equal(filters.length, 2);
  for (const [i, filter] of filters.entries()) {
    assert.ok(
      filter
        .join(" ")
        .includes("protocol ip prio 1 u32 match ip protocol 17 0xff"),
    );
    assert.ok(
      filter
        .join(" ")
        .includes(`match ip sport ${[10000, 10031][i]} 0xffff flowid 7a11:3`),
    );
    assert.ok(!filter.includes("replace"));
  }
  for (const ports of [[], [0], [65536], [1.1]])
    assert.throws(() => udpFilters(ports));
});
test("root drops cannot satisfy the media-child drop proof", () => {
  assert.equal(netemDrops(counters(0)), 0);
  assert.equal(netemDrops(counters(12)), 12);
  assert.throws(() => netemDrops('[{"kind":"prio","drops":90}]'));
  assert.throws(() => netemDrops(counters("12")));
});
test("setup and exercise failures restore only the owned root; root add failure leaves others alone", async () => {
  for (const failAt of [0, 1, 2, 4]) {
    const calls = [];
    const tc = async (...args) => {
      calls.push(args);
      if (calls.length - 1 === failAt) throw new Error("fault");
      return { stdout: counters(0) };
    };
    await assert.rejects(
      withUdpLoss(tc, [10000], async () => {
        throw new Error("exercise");
      }),
    );
    const deletions = calls.filter((c) => c[1] === "del");
    assert.equal(deletions.length, failAt === 0 ? 0 : 1);
    if (deletions.length)
      assert.deepEqual(deletions[0], [
        "qdisc",
        "del",
        "dev",
        "eth0",
        "root",
        "handle",
        "7a11:",
      ]);
  }
});
test("no matching UDP drops fails even if exercise passes; positive delta succeeds", async () => {
  for (const after of [0, 13]) {
    let reads = 0;
    const tc = async (...args) => ({
      stdout: args[0] === "-j" ? counters(reads++ ? after : 0) : "",
    });
    const attempt = withUdpLoss(tc, [10000], async () => "audio-proof");
    if (after === 0) await assert.rejects(attempt, /dropped no packets/);
    else
      assert.deepEqual(await attempt, { result: "audio-proof", dropped: 13 });
  }
});
