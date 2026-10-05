/* global URL, process, console, document, MediaStream, AudioContext, RTCPeerConnection, setInterval, clearInterval, fetch, setTimeout, clearTimeout */
import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import ts from "@typescript/typescript6";
import { chromium, firefox } from "playwright";
const base = process.argv[2];
assert.equal(new URL(base).hostname, "127.0.0.1");
const source = await readFile(
  new URL("../../src/voice/viewerLayers.ts", import.meta.url),
  "utf8",
);
const compiled = ts.transpileModule(source, {
  compilerOptions: {
    target: ts.ScriptTarget.ES2022,
    module: ts.ModuleKind.ES2022,
  },
}).outputText;
const mediaSource = await readFile(
  new URL("../../src/voice/media.ts", import.meta.url),
  "utf8",
);
const mediaAst = ts.createSourceFile(
  "media.ts",
  mediaSource,
  ts.ScriptTarget.ES2022,
  true,
);
const identityHelper = mediaAst.statements.find(
  (node) =>
    ts.isFunctionDeclaration(node) && node.name?.text === "publishedTrackIds",
);
assert.ok(identityHelper, "actual MSID binding helper exists");
const bindingCompiled = ts.transpileModule(identityHelper.getText(mediaAst), {
  compilerOptions: {
    target: ts.ScriptTarget.ES2022,
    module: ts.ModuleKind.ES2022,
  },
}).outputText;
const results = [];
for (const engine of [chromium, firefox]) {
  const browser = await engine.launch();
  try {
    const page = await browser.newPage();
    await page.goto(base);
    await page.addScriptTag({
      type: "module",
      content:
        compiled +
        "\nglobalThis.__layerModule={addLayeredVideo,ViewerLayerController};",
    });
    await page.waitForFunction(() => !!globalThis.__layerModule);
    const result = await page.evaluate(async () => {
      const wait = (ms) => new Promise((r) => setTimeout(r, ms));
      const post = (url, body) =>
        fetch(url, {
          method: "POST",
          headers: { "Content-Type": "application/json" },
          body: JSON.stringify(body),
        }).then((r) => {
          if (!r.ok) throw Error(`fixture ${r.status}`);
          return r.json();
        });
      const canvas = document.createElement("canvas");
      canvas.width = 640;
      canvas.height = 360;
      const ctx = canvas.getContext("2d");
      let frame = 0;
      const draw = () => {
        ctx.fillStyle = `rgb(${frame++ % 255},80,180)`;
        ctx.fillRect(0, 0, 640, 360);
        ctx.fillStyle = "#fff";
        ctx.fillRect((frame * 9) % 600, 20, 30, 30);
      };
      draw();
      const timer = setInterval(draw, 33),
        raw = canvas.captureStream(30),
        audio = new AudioContext(),
        tone = audio.createOscillator(),
        destination = audio.createMediaStreamDestination();
      tone.connect(destination);
      tone.start();
      await audio.resume();
      raw.addTrack(destination.stream.getAudioTracks()[0]);
      const pcs = [],
        pumps = [],
        videos = [];
      async function connect(url, publishing) {
        const pc = new RTCPeerConnection({ iceServers: [] });
        pcs.push(pc);
        if (publishing) {
          const sender = globalThis.__layerModule.addLayeredVideo(
            pc,
            raw.getVideoTracks()[0],
            raw,
          );
          if (!sender)
            throw Error("actual layered publishing helper unavailable");
          pc.addTrack(raw.getAudioTracks()[0], raw);
        } else {
          pc.addTransceiver("video", { direction: "recvonly" });
          pc.addTransceiver("audio", { direction: "recvonly" });
          const video = document.createElement("video");
          video.muted = true;
          video.autoplay = true;
          video.style = "width:640px;height:360px";
          document.body.append(video);
          videos.push(video);
          pc.ontrack = (e) => {
            if (e.track.kind === "video") {
              video.srcObject = e.streams[0] ?? new MediaStream([e.track]);
              void video.play();
            }
          };
        }
        await pc.setLocalDescription(await pc.createOffer());
        if (pc.iceGatheringState !== "complete")
          await new Promise((resolve, reject) => {
            const timeout = setTimeout(
              () => reject(Error("ICE deadline")),
              10000,
            );
            pc.onicegatheringstatechange = () => {
              if (pc.iceGatheringState === "complete") {
                clearTimeout(timeout);
                resolve();
              }
            };
          });
        const answer = await post(url, { sdp: pc.localDescription.sdp });
        await pc.setRemoteDescription({ type: "answer", sdp: answer.sdp });
        let done = false;
        const pump = (async () => {
          while (!done) {
            for (const message of await fetch(`/poll/${answer.probe}`).then(
              (r) => r.json(),
            )) {
              if (message.op === "o") {
                await pc.setRemoteDescription({
                  type: "offer",
                  sdp: message.sdp,
                });
                await pc.setLocalDescription(await pc.createAnswer());
                await post(`/answer/${answer.probe}`, {
                  sdp: pc.localDescription.sdp,
                });
              } else if (message.op === "err") throw Error(`SFU ${message.e}`);
            }
            await wait(40);
          }
        })();
        pumps.push(() => {
          done = true;
          return pump;
        });
        return { pc, ...answer };
      }
      let publisherProbe;
      const stats = async (pc) =>
        Array.from((await pc.getStats()).values())
          .filter((s) => s.type === "inbound-rtp")
          .map((s) => ({
            kind: s.kind,
            width: s.frameWidth,
            height: s.frameHeight,
            frames: s.framesDecoded,
            received: s.packetsReceived,
            lost: s.packetsLost,
            bytes: s.bytesReceived,
          }));
      const lastDecoded = new Map();
      async function decoded(
        viewer,
        expected,
        requestedHeight = expected === 90 ? 64 : 360,
        congested = false,
      ) {
        let sample;
        const before = lastDecoded.get(viewer.pc) ?? { frames: 0, audio: 0 };
        for (let n = 0; n < 100; n++) {
          await post(`/layer/${viewer.probe}`, {
            publisher: publisherProbe,
            height: requestedHeight,
            congested,
          });
          sample = await stats(viewer.pc);
          if (
            sample.some(
              (s) =>
                s.kind === "video" &&
                s.height === expected &&
                s.frames >= before.frames + 5,
            ) &&
            sample.some(
              (s) => s.kind === "audio" && s.received >= before.audio + 5,
            )
          ) {
            lastDecoded.set(viewer.pc, {
              frames: sample.find((s) => s.kind === "video").frames,
              audio: sample.find((s) => s.kind === "audio").received,
            });
            return sample;
          }
          await wait(100);
        }
        throw Error(
          `expected ${expected}px with audio, got ${JSON.stringify(sample)}`,
        );
      }
      try {
        const pub = await connect("/offer", true);
        publisherProbe = pub.probe;
        let observed, senders;
        for (let n = 0; n < 100; n++) {
          senders = Array.from((await pub.pc.getStats()).values())
            .filter((s) => s.type === "outbound-rtp" && s.kind === "video")
            .map((s) => ({
              rid: s.rid,
              width: s.frameWidth,
              height: s.frameHeight,
              frames: s.framesEncoded,
              packets: s.packetsSent,
            }));
          observed = await fetch(`/observed/${pub.probe}`).then((r) =>
            r.json(),
          );
          if (
            senders.length === 2 &&
            senders.every((s) => s.frames > 0 && s.packets > 0) &&
            observed.layers.length === 2
          )
            break;
          await wait(100);
        }
        const a = await connect(`/subscriber/${pub.probe}`, false),
          b = await connect(`/subscriber/${pub.probe}`, false);
        const hint = (v, height, congested = false) =>
          post(`/layer/${v.probe}`, {
            publisher: pub.probe,
            height,
            congested,
          });
        await hint(a, 64);
        await hint(b, 360);
        const before = await Promise.all([decoded(a, 90), decoded(b, 360)]),
          changes = [];
        for (let round = 0; round < 3; round++) {
          await hint(a, round % 2 === 0 ? 360 : 64);
          await hint(b, round % 2 === 0 ? 64 : 360);
          changes.push(
            await Promise.all([
              decoded(a, round % 2 === 0 ? 360 : 90),
              decoded(b, round % 2 === 0 ? 90 : 360),
            ]),
          );
        }
        await hint(a, 360, true);
        const congestion = await decoded(a, 90, 360, true);
        await hint(a, 360, false);
        const recovered = await decoded(a, 360);
        const encodings = pub.pc
          .getSenders()
          .filter((s) => s.track?.kind === "video")
          .flatMap((s) =>
            s.getParameters().encodings.map((e) => ({
              rid: e.rid,
              cap: e.maxBitrate ?? null,
            })),
          );
        return {
          senders,
          layers: observed.layers,
          before,
          changes,
          congestion,
          recovered,
          encodings,
          rendered: videos.map((v) => ({
            width: v.videoWidth,
            height: v.videoHeight,
          })),
        };
      } finally {
        for (const stop of pumps) await stop();
        pcs.forEach((p) => p.close());
        clearInterval(timer);
        raw.getTracks().forEach((t) => t.stop());
        tone.stop();
        await audio.close();
        videos.forEach((v) => v.remove());
      }
    });
    console.log(JSON.stringify({ browser: engine.name(), ...result }, null, 2));
    assert.deepEqual(result.layers.map((l) => l.rid).sort(), ["f", "q"]);
    assert.ok(result.encodings.every((e) => e.cap === null));
    assert.equal(result.changes.length, 3);
    results.push({
      browser: engine.name(),
      version: browser.version(),
      ...result,
    });
  } finally {
    await browser.close();
  }
}
console.log(JSON.stringify(results, null, 2));

// Cross-browser current/legacy codec coexistence and stable reusable MIDs.
const clientSource = await readFile(
  new URL("./native-peer-client.mjs", import.meta.url),
  "utf8",
);
const matrix = [];
for (const [publisherEngine, viewerEngine] of [
  [chromium, firefox],
  [firefox, chromium],
]) {
  const publisherBrowser = await publisherEngine.launch(),
    viewerBrowser = await viewerEngine.launch(),
    pages = [];
  async function client(browser, options) {
    const page = await browser.newPage();
    pages.push(page);
    await page.goto(base);
    await page.addScriptTag({
      type: "module",
      content:
        compiled +
        "\nglobalThis.__layerModule={addLayeredVideo,ViewerLayerController};",
    });
    await page.addScriptTag({
      type: "module",
      content:
        bindingCompiled + "\nglobalThis.__publishedTrackIds=publishedTrackIds;",
    });
    await page.addScriptTag({
      type: "module",
      content: clientSource + "\nglobalThis.__client=new NativePeer();",
    });
    await page.waitForFunction(
      () =>
        globalThis.__layerModule &&
        globalThis.__client &&
        globalThis.__publishedTrackIds,
    );
    const info = await page.evaluate(
      (options) => globalThis.__client.start(options),
      options,
    );
    return { page, ...info };
  }
  const watch = async (viewer, publisher, on, kind = "s") =>
    viewer.page.evaluate(
      async ({ viewer, publisher, on, kind }) => {
        const r = await fetch(`/watch/${viewer}`, {
          method: "POST",
          headers: { "Content-Type": "application/json" },
          body: JSON.stringify({ publisher, kind, on }),
        });
        if (!r.ok) throw Error(`watch ${r.status}`);
      },
      { viewer: viewer.probe, publisher: publisher.probe, on, kind },
    );
  try {
    const current = await client(publisherBrowser, {
      url: "/offer",
      kinds: ["v", "s", "l"],
    });
    let observed;
    for (let n = 0; n < 100; n++) {
      observed = await current.page.evaluate(
        (probe) => fetch(`/observed/${probe}`).then((r) => r.json()),
        current.probe,
      );
      if (observed.layers.length === 6) break;
      await new Promise((r) => setTimeout(r, 80));
    }
    console.log(
      JSON.stringify({
        matrixStage: "current",
        engine: publisherEngine.name(),
        observed,
      }),
    );
    assert.equal(
      observed.layers.length,
      6,
      "three current sources each have two real RIDs",
    );
    assert.ok(
      observed.layers.every((l) => l.codec.toLowerCase() === "video/vp8"),
    );
    const initial = await current.page.evaluate(() =>
      globalThis.__client.debug(),
    );
    assert.equal(initial.server.reserved, 3);
    assert.equal(
      initial.videos,
      0,
      "recvonly reservations emit no phantom tracks",
    );
    const legacyA = await client(viewerBrowser, {
      url: `/subscriber/${current.probe}`,
      kinds: ["s"],
      legacy: true,
      version: 2,
    });
    const legacyB = await client(viewerBrowser, {
      url: `/subscriber/${current.probe}`,
      kinds: ["s"],
      legacy: "vp9",
      version: 2,
    });
    for (const legacy of [legacyA, legacyB]) {
      let observation;
      for (let n = 0; n < 100; n++) {
        observation = await legacy.page.evaluate(
          (probe) => fetch(`/observed/${probe}`).then((r) => r.json()),
          legacy.probe,
        );
        if (observation.layers.length === 1) break;
        await new Promise((r) => setTimeout(r, 80));
      }
      console.log(
        JSON.stringify({
          matrixStage: "legacy",
          engine: viewerEngine.name(),
          observation,
        }),
      );
      assert.ok(
        observation.layers.length === 1 &&
          ["video/vp9", "video/av1", "video/h264", "video/vp8"].includes(
            observation.layers[0].codec.toLowerCase(),
          ),
        `actual default legacy publication ${JSON.stringify(observation)}`,
      );
      try {
        await legacy.page.evaluate(() => globalThis.__client.decoded(0));
      } catch (error) {
        console.log(
          JSON.stringify({
            matrixStage: "publisher-after-failed-legacy",
            debug: await current.page.evaluate(() =>
              globalThis.__client.debug(),
            ),
            stats: await current.page.evaluate(() =>
              globalThis.__client.stats(),
            ),
            observed: await current.page.evaluate(
              (probe) => fetch(`/observed/${probe}`).then((r) => r.json()),
              current.probe,
            ),
          }),
        );
        throw error;
      }
      const debug = await legacy.page.evaluate(() =>
        globalThis.__client.debug(),
      );
      const stats = await legacy.page.evaluate(() =>
        globalThis.__client.stats(),
      );
      assert.ok(
        stats.some(
          (s) =>
            s.type === "outbound-rtp" &&
            s.kind === "video" &&
            s.codec.toLowerCase() === observation.layers[0].codec.toLowerCase(),
        ),
        "native publisher still uses the accepted publication codec after SFU reoffers",
      );
      if (legacy === legacyB)
        assert.equal(observation.layers[0].codec.toLowerCase(), "video/vp9");
      assert.equal(debug.server.reserved, 1);
      assert.equal(
        debug.videos,
        1,
        "only the actual current camera is received",
      );
    }
    const currentViewer = await client(viewerBrowser, {
      url: `/subscriber/${current.probe}`,
      kinds: [],
    });
    await currentViewer.page.evaluate(() => globalThis.__client.decoded(0));
    await currentViewer.page.evaluate(() => {
      for (const video of globalThis.__client.videos)
        video.style = "width:112px;height:63px";
    });
    console.log(
      JSON.stringify({
        matrixStage: "viewer",
        debug: await currentViewer.page.evaluate(() =>
          globalThis.__client.debug(),
        ),
      }),
    );
    const hints = await currentViewer.page.evaluate(
      (publisher) => globalThis.__client.controller(publisher, "v", 63),
      { probe: current.probe, user: current.user },
    );
    assert.ok(
      hints.length > 0,
      "actual controller associates native trackIdentifier and rendered geometry",
    );
    const small = await currentViewer.page.evaluate(() =>
      globalThis.__client.decoded(90),
    );
    assert.ok(small.sample.some((s) => s.kind === "video" && s.height === 90));
    await currentViewer.page.evaluate(() => {
      for (const video of globalThis.__client.videos)
        video.style = "width:640px;height:360px";
    });
    await currentViewer.page.evaluate(
      (publisher) => globalThis.__client.controller(publisher, "v", 360),
      { probe: current.probe, user: current.user },
    );
    const full = await currentViewer.page.evaluate(
      (before) => globalThis.__client.decoded(180, before, true, true),
      small,
    );
    assert.ok(
      full.sample.some((s) => s.kind === "video" && s.height >= 180),
      "a fresh decoded full representation follows the quarter representation",
    );
    const ownSourceSwitches = [];
    for (const kind of ["s", "l"]) {
      await watch(currentViewer, current, true, kind);
      const source = `${current.user}:${kind}`;
      const initialSource = await currentViewer.page.evaluate(
        (source) =>
          globalThis.__client.decoded(0, undefined, true, false, source),
        source,
      );
      await currentViewer.page.evaluate((source) => {
        for (const video of globalThis.__client.videos)
          if (video.dataset.source === source)
            video.style = "width:112px;height:63px";
      }, source);
      await currentViewer.page.evaluate(
        ({ publisher, kind }) =>
          globalThis.__client.controller(publisher, kind, 63),
        { publisher: { probe: current.probe, user: current.user }, kind },
      );
      const sourceSmall = await currentViewer.page.evaluate(
        ({ source, before }) =>
          globalThis.__client.decoded(90, before, true, false, source),
        { source, before: initialSource },
      );
      await currentViewer.page.evaluate((source) => {
        for (const video of globalThis.__client.videos)
          if (video.dataset.source === source)
            video.style = "width:640px;height:360px";
      }, source);
      await currentViewer.page.evaluate(
        ({ publisher, kind }) =>
          globalThis.__client.controller(publisher, kind, 360),
        { publisher: { probe: current.probe, user: current.user }, kind },
      );
      const sourceFull = await currentViewer.page.evaluate(
        ({ source, before }) =>
          globalThis.__client.decoded(180, before, true, true, source),
        { source, before: sourceSmall },
      );
      ownSourceSwitches.push({
        kind,
        smallFrames: sourceSmall.frames,
        fullFrames: sourceFull.frames,
        source,
        track: sourceFull.track,
        audio: sourceFull.audio,
      });
    }
    const allSources = await currentViewer.page.evaluate(() =>
      globalThis.__client.debug(),
    );
    assert.deepEqual(
      new Set(allSources.videoSources),
      new Set(["v", "s", "l"].map((kind) => `${current.user}:${kind}`)),
      "all three actual source identities decode on separate received tracks",
    );
    let previous, baseline;
    const cycles = [];
    for (let round = 0; round < 20; round++) {
      const legacy = round % 2 === 0 ? legacyA : legacyB;
      if (previous) await watch(current, previous, false);
      await watch(current, legacy, true);
      const before = await current.page.evaluate(async () => {
        const stats = await globalThis.__client.stats();
        return {
          frames: stats
            .filter((s) => s.type === "inbound-rtp" && s.kind === "video")
            .reduce((sum, s) => sum + (s.frames ?? 0), 0),
          audio: stats
            .filter((s) => s.type === "inbound-rtp" && s.kind === "audio")
            .reduce((sum, s) => sum + (s.packets ?? 0), 0),
        };
      });
      const decoded = await current.page.evaluate(
        (before) => globalThis.__client.decoded(0, before),
        before,
      );
      const debug = await current.page.evaluate(() =>
        globalThis.__client.debug(),
      );
      if (round === 1) baseline = debug;
      if (baseline) {
        assert.equal(
          debug.native.length,
          baseline.native.length,
          "native transceiver count stable",
        );
        assert.equal(
          debug.server.transceivers.length,
          baseline.server.transceivers.length,
          "SFU transceiver count stable",
        );
        assert.deepEqual(
          debug.native.map((t) => t.mid),
          baseline.native.map((t) => t.mid),
          "subscriber MID reused",
        );
      }
      assert.equal(debug.server.reserved, 3);
      assert.ok(
        debug.server.bindings.some(
          (b) => b.source === `${legacy.user}:s` && b.binding != null,
        ),
        "the current foreign source has an actual negotiated sender binding",
      );
      assert.equal(
        debug.videos,
        1,
        "no phantom or newly allocated video receiver",
      );
      cycles.push({
        round,
        native: debug.native.length,
        server: debug.server.transceivers.length,
        frames: decoded.frames,
        audio: decoded.audio,
      });
      previous = legacy;
    }
    await watch(current, previous, false);
    const recovery = (
      await current.page.evaluate(() => globalThis.__client.debug())
    ).server.ridRecovery;
    assert.equal(
      recovery[0],
      3,
      "all three accepted publisher RID scopes remain active",
    );
    assert.equal(
      recovery[1],
      6,
      "six actual learned primary RID SSRCs remain scoped",
    );
    assert.ok(
      recovery[2] > 0,
      "actual missing headers were recovered through the public interceptor",
    );
    matrix.push({
      publisher: publisherEngine.name(),
      viewer: viewerEngine.name(),
      sourceLayers: observed.layers,
      initial,
      cycles,
      controllerHints: hints,
      ownSourceSwitches,
      recovery,
      full: {
        frames: full.frames,
        audio: full.audio,
        heights: full.sample
          .filter((s) => s.kind === "video")
          .map((s) => s.height),
      },
    });
  } finally {
    for (const page of pages)
      await page.evaluate(() => globalThis.__client?.stop());
    await publisherBrowser.close();
    await viewerBrowser.close();
  }
}
console.log(JSON.stringify({ crossBrowserLegacyMatrix: matrix }, null, 2));
