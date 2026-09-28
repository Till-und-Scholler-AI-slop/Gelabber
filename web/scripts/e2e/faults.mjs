// Faults apply only to this runner's API connections and owned object bucket.
import assert from "node:assert/strict";
import { URL } from "node:url";
import { createServer as tcpServer, createConnection } from "node:net";
import { createServer as httpServer, request } from "node:http";
import { once } from "node:events";

function loopback(url) {
  assert.ok(
    ["127.0.0.1", "localhost", "[::1]"].includes(url.hostname),
    "Fault target must be loopback",
  );
  return url;
}
async function listen(server, port) {
  server.listen({ host: "127.0.0.1", port, exclusive: true });
  await once(server, "listening");
}
export async function tcpFaultProxy(target, port = 0) {
  const url = loopback(new URL(target));
  assert.ok(url.port, "Fault target port required");
  let blocked = false;
  const sockets = new Set();
  const counters = { accepted: 0, rejected: 0, interrupted: 0 };
  const server = tcpServer((client) => {
    if (blocked) {
      counters.rejected++;
      client.destroy();
      return;
    }
    counters.accepted++;
    const upstream = createConnection({
      host: url.hostname,
      port: Number(url.port),
    });
    for (const socket of [client, upstream]) {
      sockets.add(socket);
      socket.on("error", () => {
        client.destroy();
        upstream.destroy();
      });
      socket.on("close", () => sockets.delete(socket));
    }
    client.pipe(upstream);
    upstream.pipe(client);
    client.on("close", () => upstream.destroy());
    upstream.on("close", () => client.destroy());
  });
  await listen(server, port);
  return {
    port: server.address().port,
    counters,
    block() {
      blocked = true;
      counters.interrupted += sockets.size;
      for (const s of sockets) s.destroy();
    },
    restore() {
      blocked = false;
    },
    async close() {
      blocked = false;
      for (const s of sockets) s.destroy();
      await new Promise((r) => server.close(r));
    },
  };
}
export async function storageFaultProxy(target, bucket, port = 0) {
  const url = loopback(new URL(target));
  assert.equal(url.protocol, "http:");
  assert.match(bucket, /^gb-(?:fault|e2e)-[a-f0-9-]+$/);
  const faults = new Set();
  const counters = { deleteFailures: 0, headFailures: 0, puts: 0, deletes: 0 };
  const sockets = new Set();
  const server = httpServer((incoming, outgoing) => {
    const path = new URL(incoming.url, "http://loopback").pathname;
    if (path !== `/${bucket}` && !path.startsWith(`/${bucket}/`)) {
      outgoing.writeHead(403).end();
      return;
    }
    const method = incoming.method;
    if (faults.has(method)) {
      if (method === "DELETE") counters.deleteFailures++;
      if (method === "HEAD") counters.headFailures++;
      incoming.resume();
      outgoing.writeHead(503).end();
      return;
    }
    if (method === "DELETE") counters.deletes++;
    if (method === "PUT" && path !== `/${bucket}`) counters.puts++;
    // Preserve signed Host/path/query exactly. Never log them or put them in reports.
    const upstream = request(
      {
        hostname: url.hostname,
        port: url.port,
        method,
        path: incoming.url,
        headers: incoming.headers,
      },
      (response) => {
        outgoing.writeHead(response.statusCode, response.headers);
        response.pipe(outgoing);
      },
    );
    upstream.on("error", () => {
      if (!outgoing.headersSent) outgoing.writeHead(502);
      outgoing.end();
    });
    incoming.on("aborted", () => upstream.destroy());
    outgoing.on("close", () => upstream.destroy());
    incoming.pipe(upstream);
  });
  server.on("connection", (socket) => {
    sockets.add(socket);
    socket.on("close", () => sockets.delete(socket));
  });
  await listen(server, port);
  return {
    port: server.address().port,
    counters,
    block(method) {
      assert.ok(["DELETE", "HEAD", "PUT"].includes(method));
      faults.add(method);
    },
    restore() {
      faults.clear();
    },
    async close() {
      faults.clear();
      for (const s of sockets) s.destroy();
      await new Promise((r) => server.close(r));
    },
  };
}
