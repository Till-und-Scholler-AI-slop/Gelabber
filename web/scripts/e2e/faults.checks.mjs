/* global fetch */
import { test } from "node:test";
import assert from "node:assert/strict";
import { createServer, createConnection } from "node:net";
import { createServer as httpServer } from "node:http";
import { once } from "node:events";
import { tcpFaultProxy, storageFaultProxy } from "./faults.mjs";
import { deleteTopicKeys } from "./redis-command.mjs";

test("TCP fault interrupts only proxy connections; independent control and restore survive", async () => {
  const target = createServer((s) => s.pipe(s));
  target.listen(0, "127.0.0.1");
  await once(target, "listening");
  const proxy = await tcpFaultProxy(
    `redis://127.0.0.1:${target.address().port}`,
  );
  const sockets = [];
  async function echo(port) {
    const s = createConnection(port, "127.0.0.1");
    sockets.push(s);
    await once(s, "connect");
    const data = once(s, "data");
    s.write("control");
    assert.equal(String((await data)[0]), "control");
    return s;
  }
  try {
    const proxied = await echo(proxy.port);
    const closed = once(proxied, "close");
    proxy.block();
    await closed;
    const direct = await echo(target.address().port);
    direct.destroy();
    proxy.restore();
    const restored = await echo(proxy.port);
    restored.destroy();
    assert.equal(proxy.counters.accepted, 2);
    assert.ok(proxy.counters.interrupted > 0);
  } finally {
    for (const s of sockets) s.destroy();
    await proxy.close();
    await new Promise((r) => target.close(r));
  }
});
test("S3 fault preserves signed host/path, isolates bucket and restores DELETE", async () => {
  let host,
    path,
    deletes = 0;
  const target = httpServer((req, res) => {
    host = req.headers.host;
    path = req.url;
    if (req.method === "DELETE") deletes++;
    res.writeHead(204).end();
  });
  target.listen(0, "127.0.0.1");
  await once(target, "listening");
  const proxy = await storageFaultProxy(
    `http://127.0.0.1:${target.address().port}`,
    "gb-fault-abcd",
  );
  const origin = `http://127.0.0.1:${proxy.port}`;
  try {
    assert.equal(
      (
        await fetch(
          `${origin}/gb-fault-abcd/object?synthetic-signature=memory-only`,
          { method: "PUT" },
        )
      ).status,
      204,
    );
    assert.equal(host, `127.0.0.1:${proxy.port}`);
    assert.equal(path, "/gb-fault-abcd/object?synthetic-signature=memory-only");
    proxy.block("DELETE");
    assert.equal(
      (await fetch(`${origin}/gb-fault-abcd/object`, { method: "DELETE" }))
        .status,
      503,
    );
    assert.equal(deletes, 0);
    assert.equal(proxy.counters.deleteFailures, 1);
    assert.equal(
      (await fetch(`${origin}/gb-foreign/object`, { method: "DELETE" })).status,
      403,
    );
    proxy.restore();
    assert.equal(
      (await fetch(`${origin}/gb-fault-abcd/object`, { method: "DELETE" }))
        .status,
      204,
    );
    assert.equal(deletes, 1);
  } finally {
    await proxy.close();
    target.closeAllConnections();
    await new Promise((r) => target.close(r));
  }
});
test("fault proxies reject remote targets and foreign bucket identifiers", async () => {
  await assert.rejects(tcpFaultProxy("redis://example.test:6379"));
  await assert.rejects(
    storageFaultProxy("http://127.0.0.1:9000", "production"),
  );
});

test("native RESP reset authenticates/selects explicitly and deletes only four matching owned keys", async () => {
  const commands = [];
  const target = createServer((socket) => {
    let buffer = "";
    socket.on("data", (data) => {
      buffer += String(data);
      // The narrow client waits for each response, so each complete command
      // here is isolated; fragmented writes are accumulated.
      const parts = buffer.split("\r\n");
      if (!parts[0].startsWith("*")) return;
      const size = Number(parts[0].slice(1));
      if (parts.length < size * 2 + 2) return;
      const command = Array.from({ length: size }, (_, i) => parts[i * 2 + 2]);
      commands.push(command);
      buffer = "";
      socket.write(command[0] === "DEL" ? ":4\r\n" : "+OK\r\n");
    });
  });
  target.listen(0, "127.0.0.1");
  await once(target, "listening");
  const id = "11111111-1111-4111-8111-111111111111";
  const keys = [
    `gb:n:c:${id}`,
    `gb:n:c:${id}:ep`,
    `gb:l:c:${id}`,
    `gb:n:c:${id}:delivery`,
  ];
  try {
    assert.equal(
      await deleteTopicKeys(
        `redis://default:synthetic@127.0.0.1:${target.address().port}/2`,
        keys,
      ),
      4,
    );
    assert.deepEqual(commands, [
      ["AUTH", "default", "synthetic"],
      ["SELECT", "2"],
      ["DEL", ...keys],
    ]);
    await assert.rejects(deleteTopicKeys("redis://example.test:6379", keys));
    await assert.rejects(
      deleteTopicKeys(`redis://127.0.0.1:${target.address().port}`, [
        ...keys.slice(0, 3),
        "foreign",
      ]),
    );
    assert.equal(commands.length, 3);
  } finally {
    await new Promise((r) => target.close(r));
  }
});
