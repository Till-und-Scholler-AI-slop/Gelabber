import assert from "node:assert/strict";
import { Buffer } from "node:buffer";
import { setTimeout } from "node:timers";
import { test } from "node:test";
import dgram from "node:dgram";
import { isVideoRtp, receiveLossProxy } from "./receive-loss-proxy.mjs";
const packet = (type, marker = false) =>
  Buffer.from([
    0x80,
    type | (marker ? 0x80 : 0),
    0,
    1,
    0,
    0,
    0,
    0,
    0,
    0,
    0,
    1,
    7,
  ]);
test("loss classifier preserves audio, RTCP, DTLS, STUN and incomplete packets", () => {
  const types = new Set([96, 97]);
  assert.equal(isVideoRtp(packet(96), types), true);
  assert.equal(isVideoRtp(packet(97, true), types), true);
  assert.equal(isVideoRtp(packet(111), types), false);
  assert.equal(isVideoRtp(packet(72, true), types), false);
  assert.equal(
    isVideoRtp(Buffer.from([22, 96, ...Array(20).fill(0)]), types),
    false,
  );
  assert.equal(
    isVideoRtp(Buffer.from([0, 96, ...Array(20).fill(0)]), types),
    false,
  );
  assert.equal(isVideoRtp(Buffer.from([0x80, 96]), types), false);
});
test("proxy forwards real unchanged datagrams, drops only selected video and closes its sockets", async () => {
  const server = dgram.createSocket("udp4"),
    client = dgram.createSocket("udp4");
  await new Promise((resolve) => server.bind(0, "127.0.0.1", resolve));
  const received = [];
  client.on("message", (bytes) => received.push(Buffer.from(bytes)));
  let upstreamAddress;
  server.on("message", (bytes, remote) => {
    upstreamAddress = remote;
    server.send(bytes, remote.port, remote.address);
  });
  const proxy = await receiveLossProxy({ port: server.address().port });
  try {
    const earlyVideo = packet(96);
    await new Promise((resolve, reject) =>
      client.send(earlyVideo, proxy.port, "127.0.0.1", (error) =>
        error ? reject(error) : resolve(),
      ),
    );
    const earlyDeadline = Date.now() + 2000;
    while (received.length < 1 && Date.now() < earlyDeadline)
      await new Promise((resolve) => setTimeout(resolve, 10));
    assert.deepEqual(received, [earlyVideo]);
    assert.equal(proxy.snapshot().video, 0);
    proxy.setVideoPayloadTypes([96]);
    assert.equal(
      proxy.snapshot().video,
      1,
      "early video must remain observable",
    );
    proxy.setLoss(true);
    const messages = [
      packet(96),
      packet(96),
      packet(96),
      packet(111),
      packet(72, true),
    ];
    await Promise.all(
      messages.map(
        (bytes) =>
          new Promise((resolve, reject) =>
            client.send(bytes, proxy.port, "127.0.0.1", (error) =>
              error ? reject(error) : resolve(),
            ),
          ),
      ),
    );
    const deadline = Date.now() + 2000;
    while (received.length < 5 && Date.now() < deadline)
      await new Promise((resolve) => setTimeout(resolve, 10));
    assert.ok(upstreamAddress);
    assert.equal(received.length, 5);
    assert.deepEqual(received, [
      earlyVideo,
      messages[0],
      messages[1],
      messages[3],
      messages[4],
    ]);
    assert.deepEqual(proxy.snapshot(), {
      forwarded: 5,
      video: 4,
      dropped: 1,
      foreign: 0,
    });
  } finally {
    await proxy.close();
    await Promise.all(
      [server, client].map(
        (socket) => new Promise((resolve) => socket.close(resolve)),
      ),
    );
  }
});
