import assert from "node:assert/strict";
import { createConnection } from "node:net";
import { once } from "node:events";
import { URL } from "node:url";
import { Buffer } from "node:buffer";

// Narrow RESP client: only AUTH/SELECT and deletion of explicitly owned topic
// keys. No dependency on an unpinned or missing redis-cli installation.
export async function deleteTopicKeys(target, keys) {
  const url = new URL(target);
  assert.equal(url.protocol, "redis:");
  assert.ok(["127.0.0.1", "localhost", "[::1]"].includes(url.hostname));
  assert.equal(keys.length, 4);
  const id = keys[0].match(/^gb:n:([sc]):([a-f0-9-]{36})$/);
  assert.ok(id);
  const prefix = keys[0];
  assert.deepEqual(keys, [
    prefix,
    `${prefix}:ep`,
    `gb:l:${id[1]}:${id[2]}`,
    `${prefix}:delivery`,
  ]);
  const db = url.pathname.slice(1) || "0";
  assert.match(db, /^\d+$/);
  const socket = createConnection({
    host: url.hostname,
    port: Number(url.port || 6379),
  });
  let buffer = "";
  let waiter;
  socket.on("data", (data) => {
    buffer += data.toString("utf8");
    waiter?.();
  });
  // Error details can contain private runtime values; expose only a fixed code.
  let failed = false;
  socket.on("error", () => {
    failed = true;
    waiter?.();
  });
  socket.on("close", () => {
    failed = true;
    waiter?.();
  });
  socket.setTimeout(2_000, () => socket.destroy());
  async function command(parts) {
    const encoded = parts
      .map((x) => `$${Buffer.byteLength(x)}\r\n${x}\r\n`)
      .join("");
    socket.write(`*${parts.length}\r\n${encoded}`);
    while (!buffer.includes("\r\n")) {
      assert.ok(!failed, "Owned Redis command connection failed");
      await new Promise((r) => {
        waiter = r;
      });
    }
    const at = buffer.indexOf("\r\n");
    const reply = buffer.slice(0, at);
    buffer = buffer.slice(at + 2);
    assert.ok(
      reply.startsWith("+") || reply.startsWith(":"),
      "Owned Redis command rejected",
    );
    return reply.slice(1);
  }
  try {
    await once(socket, "connect");
    if (url.password)
      await command(
        url.username
          ? [
              "AUTH",
              decodeURIComponent(url.username),
              decodeURIComponent(url.password),
            ]
          : ["AUTH", decodeURIComponent(url.password)],
      );
    if (db !== "0") await command(["SELECT", db]);
    const count = Number(await command(["DEL", ...keys]));
    assert.ok(
      Number.isSafeInteger(count) && count > 0 && count <= 4,
      "Owned Redis topic control missing",
    );
    return count;
  } finally {
    socket.destroy();
  }
}
