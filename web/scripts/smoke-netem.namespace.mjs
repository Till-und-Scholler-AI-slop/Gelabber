// Run ONLY inside a newly created disposable network namespace:
// GELABBER_NETEM_HOST_NS="$(readlink /proc/self/ns/net)" unshare -Urn sh -c
// 'ip link set lo up; node web/scripts/smoke-netem.namespace.mjs'
/* global console, process */
import assert from "node:assert/strict";
import { Buffer } from "node:buffer";
import { performance } from "node:perf_hooks";
import { execFile } from "node:child_process";
import { promisify } from "node:util";
import { createSocket } from "node:dgram";
import { createServer, createConnection } from "node:net";
import { once } from "node:events";
import { setTimeout as pause } from "node:timers/promises";
import { withUdpLoss } from "./smoke-netem.mjs";

// Require a different netns from the launching shell/host, before touching lo.
const exec = promisify(execFile);
const { stdout } = await exec("readlink", ["/proc/self/ns/net"]);
assert.match(
  process.env.GELABBER_NETEM_HOST_NS ?? "",
  /^net:\[\d+\]$/,
  "Supply the launching namespace ID before unshare",
);
assert.notEqual(
  stdout.trim(),
  process.env.GELABBER_NETEM_HOST_NS,
  "Refusing host/shared network namespace",
);
const tcp = createServer((socket) => socket.pipe(socket));
const receive = createSocket("udp4");
const send = createSocket("udp4");
let client;
let received = 0;
receive.on("message", () => received++);
try {
  tcp.listen(0, "127.0.0.1");
  await once(tcp, "listening");
  receive.bind(0, "127.0.0.1");
  send.bind(0, "127.0.0.1");
  await Promise.all([once(receive, "listening"), once(send, "listening")]);
  // Same numerical source port, different transport: TCP must not be shaped.
  client = createConnection({
    host: "127.0.0.1",
    port: tcp.address().port,
    localAddress: "127.0.0.1",
    localPort: send.address().port,
  });
  await once(client, "connect");
  let maxTcpMs = 0;
  const { dropped } = await withUdpLoss(
    async (...args) => exec("tc", args),
    [send.address().port],
    async () => {
      for (let i = 0; i < 100; i++) {
        const start = performance.now();
        const data = once(client, "data");
        const payload = Buffer.from(`authority-check-${i}`);
        client.write(payload);
        assert.deepEqual((await data)[0], payload);
        maxTcpMs = Math.max(maxTcpMs, performance.now() - start);
        for (let j = 0; j < 10; j++)
          await new Promise((resolve, reject) =>
            send.send(
              Buffer.from("media"),
              receive.address().port,
              "127.0.0.1",
              (error) => (error ? reject(error) : resolve()),
            ),
          );
        await pause(2);
      }
      await pause(100);
      assert.ok(maxTcpMs < 500, "Control TCP exceeded authority deadline");
      assert.ok(received > 0 && received < 1000, "No actual UDP packet loss");
    },
    "lo",
  );
  console.log(
    JSON.stringify({
      udpSent: 1000,
      udpReceived: received,
      netemChildDrops: dropped,
      tcpEchoes: 100,
      maxTcpMs: Math.ceil(maxTcpMs),
      authorityDeadlineMs: 500,
      scope: "disposable netns kernel boundary; not SFU/relay CI acceptance",
    }),
  );
} finally {
  client?.destroy();
  receive.close();
  send.close();
  await new Promise((resolve) => tcp.close(resolve));
}
process.exitCode = 0;
