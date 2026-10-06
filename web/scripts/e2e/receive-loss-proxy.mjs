// Owned loopback UDP forwarding for short receiver-loss acceptance. SRTP is
// forwarded unchanged; only selected video RTP payload types may be dropped.
import assert from "node:assert/strict";
import dgram from "node:dgram";

export function isVideoRtp(packet, payloadTypes) {
  if (packet.length < 12 || (packet[0] & 0xc0) !== 0x80) return false;
  // RFC 5761: the unmasked RTCP packet-type range is reserved for RTCP.
  if (packet[1] >= 192 && packet[1] <= 223) return false;
  return payloadTypes.has(packet[1] & 0x7f);
}
export async function receiveLossProxy({ port, address = "127.0.0.1" }) {
  assert.equal(
    address,
    "127.0.0.1",
    "Only an owned loopback media server is allowed",
  );
  assert.ok(Number.isInteger(port) && port > 1024 && port <= 65535);
  const front = dgram.createSocket("udp4"),
    upstream = dgram.createSocket("udp4");
  const counters = { forwarded: 0, video: 0, dropped: 0, foreign: 0 };
  let client = null,
    loss = false,
    sequence = 0;
  let payloadTypes = new Set();
  const packetsByPayload = new Map();
  let failure;
  const fail = (error) => {
    failure ??= error;
  };
  front.on("error", fail);
  upstream.on("error", fail);
  const bind = (socket) =>
    new Promise((resolve, reject) => {
      socket.once("error", reject);
      socket.bind(0, "127.0.0.1", () => {
        socket.removeListener("error", reject);
        resolve();
      });
    });
  await Promise.all([bind(front), bind(upstream)]);
  front.on("message", (packet, remote) => {
    client ??= { port: remote.port, address: remote.address };
    if (remote.port !== client.port || remote.address !== client.address) {
      counters.foreign++;
      return;
    }
    upstream.send(packet, port, address, fail);
  });
  upstream.on("message", (packet, remote) => {
    if (remote.port !== port || remote.address !== address) {
      counters.foreign++;
      return;
    }
    if (!client) return;
    if (
      packet.length >= 12 &&
      (packet[0] & 0xc0) === 0x80 &&
      !(packet[1] >= 192 && packet[1] <= 223)
    ) {
      const type = packet[1] & 0x7f;
      packetsByPayload.set(type, (packetsByPayload.get(type) ?? 0) + 1);
    }
    if (isVideoRtp(packet, payloadTypes)) {
      counters.video++;
      if (loss && ++sequence % 3 === 0) {
        counters.dropped++;
        return;
      }
    }
    counters.forwarded++;
    front.send(packet, client.port, client.address, fail);
  });
  return {
    port: front.address().port,
    setVideoPayloadTypes(values) {
      assert.ok(
        values.length > 0 &&
          values.every(
            (value) => Number.isInteger(value) && value >= 0 && value <= 127,
          ),
      );
      payloadTypes = new Set(values);
      // Include packets observed before the actual Consumer exposed its codec
      // types, so a pre-Ready leak cannot disappear from the paused check.
      counters.video = [...payloadTypes].reduce(
        (count, type) => count + (packetsByPayload.get(type) ?? 0),
        0,
      );
    },
    setLoss(value) {
      loss = value;
    },
    resetClient() {
      // The owner calls this only after its browser's old transports closed.
      client = null;
    },
    snapshot() {
      if (failure) throw failure;
      return { ...counters };
    },
    async close() {
      await Promise.all(
        [front, upstream].map(
          (socket) => new Promise((resolve) => socket.close(resolve)),
        ),
      );
      if (failure) throw failure;
    },
  };
}
