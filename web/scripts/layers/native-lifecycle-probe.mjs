/* global fetch, process, URL, console, setTimeout */
import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import ts from "@typescript/typescript6";
import { chromium, firefox } from "playwright";

const base = process.argv[2];
assert.equal(new URL(base).hostname, "127.0.0.1");
const compile = (source) =>
  ts.transpileModule(source, {
    compilerOptions: {
      target: ts.ScriptTarget.ES2022,
      module: ts.ModuleKind.ES2022,
    },
  }).outputText;
const layerSource = await readFile(
  new URL("../../src/voice/viewerLayers.ts", import.meta.url),
  "utf8",
);
const mediaSource = await readFile(
  new URL("../../src/voice/media.ts", import.meta.url),
  "utf8",
);
const ast = ts.createSourceFile(
  "media.ts",
  mediaSource,
  ts.ScriptTarget.ES2022,
  true,
);
const helper = ast.statements.find(
  (node) =>
    ts.isFunctionDeclaration(node) && node.name?.text === "publishedTrackIds",
);
assert.ok(helper, "product MSID helper exists");
const peerSource = await readFile(
  new URL("./native-peer-client.mjs", import.meta.url),
  "utf8",
);
async function client(browser, options) {
  const page = await browser.newPage();
  await page.goto(base);
  await page.addScriptTag({
    type: "module",
    content:
      compile(layerSource) +
      "\nglobalThis.__layerModule={addLayeredVideo,ViewerLayerController};",
  });
  await page.addScriptTag({
    type: "module",
    content:
      compile(helper.getText(ast)) +
      "\nglobalThis.__publishedTrackIds=publishedTrackIds;",
  });
  await page.addScriptTag({
    type: "module",
    content: peerSource + "\nglobalThis.__client=new NativePeer();",
  });
  await page.waitForFunction(
    () =>
      globalThis.__layerModule &&
      globalThis.__publishedTrackIds &&
      globalThis.__client,
  );
  return {
    page,
    ...(await page.evaluate(
      (options) => globalThis.__client.start(options),
      options,
    )),
  };
}
const watch = async (viewer, publisher, kind, on) =>
  viewer.page.evaluate(
    async (body) => {
      const r = await fetch(`/watch/${globalThis.__client.probe}`, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify(body),
      });
      if (!r.ok) throw Error(`watch ${r.status}`);
      await globalThis.__client.stable();
    },
    { publisher: publisher.probe, kind, on },
  );
async function observed(publisher) {
  return publisher.page.evaluate(
    (probe) => fetch(`/observed/${probe}`).then((r) => r.json()),
    publisher.probe,
  );
}
async function awaitPublication(publisher, source, present) {
  let result;
  for (let n = 0; n < 100; n++) {
    result = await observed(publisher);
    const publication = result.publications.find((p) => p.source === source);
    if (present ? publication?.received >= 5 : publication === undefined)
      return result;
    await new Promise((r) => setTimeout(r, 80));
  }
  throw Error(
    `publication ${source} expected ${present}: ${JSON.stringify(result)}`,
  );
}
const matrix = [];
for (const [publisherEngine, viewerEngine] of [
  [chromium, firefox],
  [firefox, chromium],
]) {
  const publisherBrowser = await publisherEngine.launch(),
    viewerBrowser = await viewerEngine.launch();
  let publisher, viewer;
  try {
    publisher = await client(publisherBrowser, {
      url: "/offer",
      kinds: ["v", "s", "l"],
    });
    viewer = await client(viewerBrowser, {
      url: `/subscriber/${publisher.probe}`,
      kinds: [],
    });
    for (const kind of ["s", "l"]) await watch(viewer, publisher, kind, true);
    for (const kind of ["v", "s", "l"]) {
      await viewer.page.evaluate(
        async (source) => globalThis.__client.freshSource(source, []),
        `${publisher.user}:${kind}`,
      );
    }
    const cycles = [];
    // Allow sender header acknowledgements to settle: resumed capture must
    // succeed even when the browser no longer repeats its original MID/RIDs.
    await new Promise((resolve) => setTimeout(resolve, 3000));
    const initialPublisher = await publisher.page.evaluate(() =>
      globalThis.__client.debug(),
    );
    const initialViewer = await viewer.page.evaluate(() =>
      globalThis.__client.debug(),
    );
    const count = Number(process.env.GELABBER_LAYER_CAPTURE_CYCLES ?? 20);
    assert.ok(Number.isSafeInteger(count) && count >= 3 && count <= 100);
    for (let round = 0; round < count; round++) {
      const kind = ["v", "s", "l"][round % 3];
      const source = `${publisher.user}:${kind}`;
      const before = await viewer.page.evaluate(() =>
        globalThis.__client.stats(),
      );
      if (kind !== "v") await watch(viewer, publisher, kind, false);
      const stopped = await publisher.page.evaluate(
        ({ kind, strategy }) => globalThis.__client.stopSource(kind, strategy),
        {
          kind,
          strategy: process.env.GELABBER_LAYER_STOP_STRATEGY ?? "replace",
        },
      );
      assert.equal(stopped.stopped, "ended", "old capture is truly stopped");
      const absent = await awaitPublication(publisher, source, false);
      const stoppedState = await publisher.page.evaluate(() =>
        globalThis.__client.debug(),
      );
      assert.equal(
        stoppedState.server.ridRecovery[0],
        2,
        "stopped source no longer has a recovery scope",
      );
      console.log(
        JSON.stringify({
          stage: "source-stopped",
          source,
          stopped,
          receivers: stoppedState.server.receivers,
          recovery: stoppedState.server.ridRecovery,
        }),
      );
      const restarted = await publisher.page.evaluate(
        (kind) => globalThis.__client.restartSource(kind),
        kind,
      );
      assert.notEqual(
        restarted.capture,
        stopped.capture,
        "a fresh native captured track replaces the stopped one",
      );
      assert.equal(
        restarted.mid,
        stopped.mid,
        "product sender reuse keeps the publication MID",
      );
      assert.equal(restarted.ready, "live");
      if (kind !== "v") await watch(viewer, publisher, kind, true);
      const fresh = await viewer.page.evaluate(
        ({ source, before }) => globalThis.__client.freshSource(source, before),
        { source, before },
      );
      const present = await awaitPublication(publisher, source, true);
      const restored = await publisher.page.evaluate(() =>
        globalThis.__client.debug(),
      );
      assert.equal(
        restored.server.ridRecovery[0],
        3,
        "all three current source scopes are restored",
      );
      assert.equal(
        restored.server.ridRecovery[1],
        6,
        "both current primary encodings are safely bound again",
      );
      assert.ok(
        restored.server.ridBoundSeeds > 0,
        "current public receiver rebind was exercised",
      );
      assert.deepEqual(
        restored.native.map((t) => t.mid),
        initialPublisher.native.map((t) => t.mid),
        "twenty fresh capture restarts retain bounded native MIDs",
      );
      assert.equal(
        restored.server.transceivers.length,
        initialPublisher.server.transceivers.length,
      );
      const viewerState = await viewer.page.evaluate(() =>
        globalThis.__client.debug(),
      );
      assert.equal(
        viewerState.native.length,
        initialViewer.native.length,
        "subscriber transceivers are reused after each new publication",
      );
      assert.equal(
        viewerState.server.transceivers.length,
        initialViewer.server.transceivers.length,
      );
      assert.ok(
        fresh.video.frames >= 5 && fresh.video.packets >= 5 && fresh.audio > 0,
      );
      cycles.push({
        kind,
        stopped,
        restarted,
        fresh,
        absent: absent.publications,
        present: present.publications,
        recovery: restored.server.ridRecovery,
        nativeMids: restored.native.map((t) => t.mid),
      });
      console.log(
        JSON.stringify({
          stage: "source-restarted",
          publisher: publisherEngine.name(),
          kind,
          ...cycles.at(-1),
        }),
      );
    }
    const final = await viewer.page.evaluate(() => globalThis.__client.debug());
    const activeSourceSsrcs = new Set(
      final.server.bindings
        .filter((b) =>
          ["v", "s", "l"].some((k) => b.source === `${publisher.user}:${k}`),
        )
        .map((b) => b.ssrc),
    );
    assert.equal(
      activeSourceSsrcs.size,
      3,
      "all three restarted sources have separate current receiver bindings",
    );
    matrix.push({
      publisher: publisherEngine.name(),
      publisherVersion: publisherBrowser.version(),
      viewer: viewerEngine.name(),
      viewerVersion: viewerBrowser.version(),
      cycles,
    });
  } catch (error) {
    if (publisher)
      console.log(
        JSON.stringify({
          stage: "restart-failure",
          publisher: await publisher.page.evaluate(() =>
            globalThis.__client.debug(),
          ),
          observed: await observed(publisher),
          stats: await publisher.page.evaluate(() =>
            globalThis.__client.stats(),
          ),
        }),
      );
    throw error;
  } finally {
    for (const peer of [viewer, publisher])
      if (peer) await peer.page.evaluate(() => globalThis.__client.stop());
    await publisherBrowser.close();
    await viewerBrowser.close();
  }
}
console.log(JSON.stringify({ nativeSourceRestartMatrix: matrix }, null, 2));
