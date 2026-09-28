// Netem owns one temporary qdisc tree in the disposable media namespace.
// Everything defaults to the unshaped band; only selected SFU UDP ports match.
import assert from "node:assert/strict";

export function udpFilters(ports, device = "eth0") {
  assert.ok(ports.length > 0, "No selected SFU UDP ports");
  assert.match(device, /^[a-z0-9]+$/);
  return [...new Set(ports)].map((port) => {
    assert.ok(Number.isInteger(port) && port > 0 && port <= 65535);
    return [
      "filter",
      "add",
      "dev",
      device,
      "parent",
      "7a11:",
      "protocol",
      "ip",
      "prio",
      "1",
      "u32",
      "match",
      "ip",
      "protocol",
      "17",
      "0xff",
      "match",
      "ip",
      "sport",
      String(port),
      "0xffff",
      "flowid",
      "7a11:3",
    ];
  });
}

export function netemDrops(stdout) {
  const child = JSON.parse(stdout).filter(
    (q) => q.kind === "netem" && q.handle === "7a12:" && q.parent === "7a11:3",
  );
  assert.equal(child.length, 1, "Missing or ambiguous owned UDP netem child");
  assert.ok(
    Number.isSafeInteger(child[0].drops) && child[0].drops >= 0,
    "Missing UDP netem drop counter",
  );
  return child[0].drops;
}

export async function withUdpLoss(tc, ports, exercise, device = "eth0") {
  // Validate before creating a root. add (never replace) refuses an existing root.
  const filters = udpFilters(ports, device);
  await tc(
    "qdisc",
    "add",
    "dev",
    device,
    "root",
    "handle",
    "7a11:",
    "prio",
    "bands",
    "3",
    "priomap",
    ...Array(16).fill("0"),
  );
  try {
    await tc(
      "qdisc",
      "add",
      "dev",
      device,
      "parent",
      "7a11:3",
      "handle",
      "7a12:",
      "netem",
      "loss",
      "8%",
    );
    for (const args of filters) await tc(...args);
    const drops = async () =>
      netemDrops((await tc("-j", "-s", "qdisc", "show", "dev", device)).stdout);
    const before = await drops();
    const result = await exercise();
    const dropped = (await drops()) - before;
    assert.ok(dropped > 0, "Selected SFU UDP netem dropped no packets");
    return { result, dropped };
  } finally {
    // Remove only the tree created above, including on setup/exercise failure.
    await tc("qdisc", "del", "dev", device, "root", "handle", "7a11:");
  }
}
