// Real pixels of the desktop app's in-page video, without the app: headless
// Chromium (WebGL on SwiftShader, no window) loads the bundled tile, feed and
// renderer; a fake `window.__TAURI_INTERNALS__` plays the app and serves
// synthetic I420 packets; the tiles' canvases are read back and compared.
// No server, account or network. WebKitGTK itself is not covered here.
// Run in web/: node scripts/smoke-native-video.mjs
/* global process, console, window, document, URL, HTMLCanvasElement, requestAnimationFrame, getComputedStyle */
import assert from "node:assert/strict";
import { mkdtemp, readFile, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join, normalize } from "node:path";
import { fileURLToPath } from "node:url";
import { chromium } from "playwright";
import { build } from "vite";

const web = fileURLToPath(new URL("..", import.meta.url));
const ORIGIN = "http://native-video.test";
/** Largest difference of a colour channel from the expected value. */
const TOLERANCE = 3;

const WHITE = [255, 255, 255];
const BLACK = [0, 0, 0];
const RED = [255, 0, 0];
const GREEN = [0, 255, 0];
const BLUE = [0, 0, 255];
const BARS = [
  WHITE,
  [255, 255, 0],
  [0, 255, 255],
  GREEN,
  [255, 0, 255],
  RED,
  BLUE,
  BLACK,
];

/** The desktop app as the page sees it. Runs in the page before its code;
 * `window.smokeApp` is the runner's handle on it. */
function desktopApp(options) {
  /** A packet as the app sends it: 32 byte header, then I420 planes of
   * vertical bars or of four quadrants in the given colours. */
  function encode(spec, seq) {
    const { width, height, colours, quadrants, bt709, turns } = spec;
    const chromaWidth = (width + 1) >> 1;
    const chromaHeight = (height + 1) >> 1;
    const bytes = new Uint8Array(
      32 + width * height + 2 * chromaWidth * chromaHeight,
    );
    const header = new DataView(bytes.buffer);
    bytes.set([0x47, 0x46, 0x52, 0x31]); // "GFR1"
    header.setUint16(4, 32, true);
    header.setUint8(6, 0); // I420
    header.setUint8(7, (bt709 ? 1 : 0) | ((turns ?? 0) << 1));
    header.setUint32(8, width, true);
    header.setUint32(12, height, true);
    header.setUint32(16, seq, true);
    header.setBigInt64(24, BigInt(seq) * 33_333n, true);
    // Limited-range YCbCr from RGB, by the standards' own definition.
    const [kr, kb] = bt709 ? [0.2126, 0.0722] : [0.299, 0.114];
    const coded = colours.map((colour) => {
      const [r, g, b] = colour.map((value) => value / 255);
      const luma = kr * r + (1 - kr - kb) * g + kb * b;
      return [
        Math.round(16 + 219 * luma),
        Math.round(128 + (224 * (b - luma)) / (2 * (1 - kb))),
        Math.round(128 + (224 * (r - luma)) / (2 * (1 - kr))),
      ];
    });
    const at = (x, y) =>
      quadrants
        ? coded[(2 * y >= height ? 2 : 0) + (2 * x >= width ? 1 : 0)]
        : coded[Math.floor((x * coded.length) / width)];
    let index = 32;
    for (let y = 0; y < height; y++)
      for (let x = 0; x < width; x++) bytes[index++] = at(x, y)[0];
    for (const plane of [1, 2])
      for (let y = 0; y < chromaHeight; y++)
        for (let x = 0; x < chromaWidth; x++)
          bytes[index++] = at(
            Math.min(width - 1, 2 * x),
            Math.min(height - 1, 2 * y),
          )[plane];
    return bytes.buffer;
  }

  const views = new Map();
  let callbacks = 0;
  const app = {
    /** Every command the page invoked. */
    log: [],
    /** What a consumer or source shows, by "consumer:1" / "source:7". */
    sources: new Map(),
    /** Contract breaches of the page. */
    violations: [],
    /** WebGL contexts the page created. */
    contexts: [],
    /** A new frame for every view of `target`. */
    push(target, spec) {
      app.sources.set(target, spec);
      for (const view of views.values()) {
        if (view.target !== target || view.closed) continue;
        view.spec = spec;
        view.seq++;
        answer(view);
      }
    },
    views: (target) =>
      [...views.values()].filter((view) => view.target === target),
    /** True when the page holds the newest frame of every open view of
     * `target` and waits for the next. */
    caughtUp(target) {
      const open = app.views(target).filter((view) => !view.closed);
      return (
        open.length > 0 &&
        open.every((view) => view.seq > 0 && view.waiting?.after === view.seq)
      );
    },
    named: (command) =>
      app.log.filter((call) => call.command === command).map((c) => c.args),
  };
  window.smokeApp = app;

  /** The long poll: the waiting request gets the frame it has not seen. */
  function answer(view) {
    const waiting = view.waiting;
    if (!waiting || view.seq === 0 || waiting.after === view.seq) return;
    view.waiting = null;
    const packet = encode(view.spec, view.seq);
    // Tauri's postMessage fallback hands raw bytes over as a number array.
    waiting.resolve(options.json ? Array.from(new Uint8Array(packet)) : packet);
  }

  const getContext = HTMLCanvasElement.prototype.getContext;
  HTMLCanvasElement.prototype.getContext = function (kind, ...rest) {
    const webgl = String(kind).startsWith("webgl");
    if (webgl && options.noWebgl) return null;
    const context = getContext.call(this, kind, ...rest);
    if (webgl && context && !app.contexts.includes(context))
      app.contexts.push(context);
    return context;
  };

  window.__TAURI_INTERNALS__ = {
    transformCallback(callback) {
      window[`_${++callbacks}`] = callback;
      return callbacks;
    },
    unregisterCallback(id) {
      delete window[`_${id}`];
    },
    async invoke(command, args = {}) {
      app.log.push({ command, args: JSON.parse(JSON.stringify(args)) });
      const view = views.get(args.view);
      switch (command) {
        case "media_info":
          return {
            abi: options.features ? 8 : 7,
            version: options.features ? "0.6.0" : "0.5.2",
            platform: "linux",
            ...(options.features ? { features: options.features } : {}),
          };
        case "media_view_open": {
          const target =
            args.consumer === undefined
              ? `source:${args.source}`
              : `consumer:${args.consumer}`;
          const spec = app.sources.get(target) ?? null;
          const id = views.size + 1;
          views.set(id, { id, target, spec, seq: spec ? 1 : 0, waiting: null });
          return { view: id };
        }
        case "media_view_frame":
          if (!view || view.closed) throw "view closed";
          if (view.waiting)
            app.violations.push(`view ${view.id}: two requests in flight`);
          return new Promise((resolve, reject) => {
            view.waiting = { after: args.after, resolve, reject };
            answer(view);
          });
        case "media_view_configure":
          if (!view || view.closed) throw "unknown view";
          return null;
        case "media_view_close":
          if (view && !view.closed) {
            view.closed = true;
            view.waiting?.reject("view closed");
            view.waiting = null;
          }
          return null;
        case "media_viewer_open":
        case "media_viewer_close":
          return null;
        default:
          throw `${command} not allowed`;
      }
    },
  };
}

// The production transforms of `vite build` on the page module.
const out = await mkdtemp(join(tmpdir(), "gelabber-native-video-"));
await build({
  root: web,
  logLevel: "error",
  build: {
    outDir: out,
    emptyOutDir: true,
    manifest: true,
    copyPublicDir: false,
    rollupOptions: { input: join(web, "scripts/smoke-native-video.page.mjs") },
  },
});
const manifest = JSON.parse(
  await readFile(join(out, ".vite/manifest.json"), "utf8"),
);
const entry = Object.values(manifest).find((chunk) => chunk.isEntry);
assert.ok(entry?.css?.length, "the page module was not built with its styles");
const html = `<!doctype html><html lang="de"><head><meta charset="utf-8">${entry.css
  .map((file) => `<link rel="stylesheet" href="/${file}">`)
  .join("")}</head><body><div id="root"></div>
<script type="module" src="/${entry.file}"></script></body></html>`;

const browser = await chromium.launch({
  executablePath:
    process.env.GELABBER_NATIVE_VIDEO_BROWSER_EXECUTABLE || undefined,
  // Never a window: this runs on developers' desktops.
  headless: true,
  args: [
    "--use-angle=swiftshader",
    "--enable-unsafe-swiftshader",
    "--ignore-gpu-blocklist",
  ],
});

/** The renderer sizes its conversion canvas by engine (frames.ts): as in
 * Chromium, which this is and WebView2 has, and as in the Linux app's
 * WebKitGTK, whose name is all this Chromium takes from it. */
const ENGINES = [
  { name: "Chromium", userAgent: undefined, canvasGrows: true },
  {
    name: "WebKitGTK",
    userAgent:
      "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/60.5 Safari/605.1.15",
    canvasGrows: false,
  },
];

/** A page whose desktop app is `desktopApp(options)`. */
async function open(options, userAgent) {
  // The device pixel ratio WebKitGTK reports on a scaled display.
  const context = await browser.newContext({
    viewport: { width: 1280, height: 900 },
    deviceScaleFactor: 2,
    userAgent,
  });
  const page = await context.newPage();
  const problems = [];
  const warnings = [];
  page.on("pageerror", (error) => problems.push(error.message));
  page.on("console", (message) => {
    if (message.type() === "error") problems.push(message.text());
    // Chromium's advice about this script's own getImageData calls aside.
    if (message.type() === "warning" && !message.text().startsWith("Canvas2D:"))
      warnings.push(message.text());
  });
  await page.route("**/*", (route) => {
    const url = new URL(route.request().url());
    if (url.origin !== ORIGIN) return route.abort();
    if (url.pathname === "/")
      return route.fulfill({ contentType: "text/html", body: html });
    if (url.pathname === "/favicon.ico") return route.fulfill({ status: 204 });
    const file = normalize(join(out, url.pathname));
    return file.startsWith(out) ? route.fulfill({ path: file }) : route.abort();
  });
  await page.addInitScript(desktopApp, options);
  await page.goto(`${ORIGIN}/`);
  await page.waitForFunction(() => window.smoke);
  return { page, context, problems, warnings };
}

const show = (page, tiles) =>
  page.evaluate((list) => window.smoke.show(list), tiles);
const push = (page, target, spec) =>
  page.evaluate(([to, what]) => window.smokeApp.push(to, what), [target, spec]);
const named = (page, command) =>
  page.evaluate((name) => window.smokeApp.named(name), command);
const frames = (page, count = 2) =>
  page.evaluate(async (left) => {
    while (left-- > 0) await new Promise(requestAnimationFrame);
  }, count);

/** Until the page has drawn the newest frame of `target`. */
async function painted(page, target) {
  await page.waitForFunction((to) => window.smokeApp.caughtUp(to), target);
  await frames(page);
}

/** Bitmap size, style and the colours at `points` of a track's canvas. */
function read(page, id, points = [], index = 0) {
  return page.evaluate(
    ([track, at, nth]) => {
      const canvas = document.querySelectorAll(
        `canvas[data-native-track="${track}"]`,
      )[nth];
      const context = canvas.getContext("2d");
      const style = getComputedStyle(canvas);
      const box = canvas.getBoundingClientRect();
      return {
        size: [canvas.width, canvas.height],
        box: [box.width, box.height],
        shown: style.opacity === "1",
        fit: style.objectFit,
        // Tailwind mirrors with the `scale` property.
        mirrored: style.scale === "-1 1",
        pixels: at.map(([x, y]) => [
          ...context.getImageData(x, y, 1, 1).data.slice(0, 3),
        ]),
      };
    },
    [id, points, index],
  );
}

let worst = 0;
function same(got, want, what) {
  const off = Math.max(...got.map((value, at) => Math.abs(value - want[at])));
  worst = Math.max(worst, off);
  assert.ok(off <= TOLERANCE, `${what}: got ${got}, expected ${want}`);
}

/** Centres of `count` vertical bars on a picture of `width` x `height`. */
const centres = (count, width, height) =>
  Array.from({ length: count }, (_, bar) => [
    Math.floor(((bar + 0.5) * width) / count),
    height >> 1,
  ]);

async function bars(page, id, target, spec, what) {
  await push(page, target, spec);
  await painted(page, target);
  const { width, height, colours } = spec;
  const got = await read(page, id, centres(colours.length, width, height));
  assert.deepEqual(got.size, [width, height], `${what}: bitmap size`);
  assert.ok(got.shown, `${what}: the canvas is visible`);
  colours.forEach((colour, bar) =>
    same(got.pixels[bar], colour, `${what}, bar ${bar}`),
  );
}

/** Corners of a picture that is turned by `turns` quarter turns. */
async function turned(page, id, target, turns, corners, what) {
  const spec = { width: 640, height: 360, quadrants: true, turns };
  await push(page, target, { ...spec, colours: [RED, GREEN, BLUE, WHITE] });
  await painted(page, target);
  const [wide, high] = turns % 2 ? [360, 640] : [640, 360];
  const got = await read(page, id, [
    [20, 20],
    [wide - 20, 20],
    [20, high - 20],
    [wide - 20, high - 20],
  ]);
  assert.deepEqual(got.size, [wide, high], `${what}: bitmap size`);
  corners.forEach((colour, corner) =>
    same(got.pixels[corner], colour, `${what}, corner ${corner}`),
  );
}

try {
  // A 0.6 app: remote and own video in the tiles.
  for (const engine of ENGINES) {
    console.log(`With the conversion canvas of ${engine.name}:`);
    const { page, context, problems, warnings } = await open(
      { features: ["video-frames"] },
      engine.userAgent,
    );
    const remote = "consumer:1";
    const ids = await show(page, [
      { name: "Alex", consumer: 1, width: 480, screen: true },
    ]);
    await bars(
      page,
      ids.Alex,
      remote,
      { width: 640, height: 360, colours: BARS },
      "BT.601",
    );
    // 480x270 CSS pixels at a device pixel ratio of 2.
    assert.deepEqual(await named(page, "media_view_open"), [
      { consumer: 1, maxWidth: 960, maxHeight: 540 },
    ]);
    assert.equal(
      await page.evaluate(
        (id) => window.smoke.renderedVideoHeight(id),
        ids.Alex,
      ),
      540,
    );
    console.log("PASS: BT.601 colour bars from a remote stream");

    await bars(
      page,
      ids.Alex,
      remote,
      { width: 1280, height: 720, colours: BARS, bt709: true },
      "BT.709",
    );
    console.log("PASS: BT.709 colour bars, picture size follows the frames");

    await bars(
      page,
      ids.Alex,
      remote,
      { width: 321, height: 181, colours: [RED, GREEN, BLUE, WHITE, BLACK] },
      "odd size",
    );
    const edge = await read(page, ids.Alex, [
      [0, 0],
      [320, 0],
      [0, 180],
      [320, 180],
    ]);
    [RED, BLACK, RED, BLACK].forEach((colour, corner) =>
      same(edge.pixels[corner], colour, `odd size, corner ${corner}`),
    );
    console.log("PASS: odd frame sizes keep their rows and edges");

    await turned(page, ids.Alex, remote, 0, [RED, GREEN, BLUE, WHITE], "0°");
    await turned(page, ids.Alex, remote, 1, [BLUE, RED, WHITE, GREEN], "90°");
    await turned(page, ids.Alex, remote, 2, [WHITE, BLUE, GREEN, RED], "180°");
    await turned(page, ids.Alex, remote, 3, [GREEN, WHITE, RED, BLUE], "270°");
    console.log("PASS: the rotation flag turns the picture clockwise");

    // The own camera (4:3) and the own Live next to the watched stream.
    await push(page, "source:7", { width: 640, height: 480, colours: BARS });
    await push(page, "source:8", { width: 640, height: 360, colours: BARS });
    const tiles = [
      { name: "Alex", consumer: 1, width: 480, screen: true },
      { name: "Ich", source: 7, width: 320, mirror: true },
      { name: "Mein Live", source: 8, width: 320, screen: true, live: true },
    ];
    Object.assign(ids, await show(page, tiles));
    await painted(page, "source:7");
    await painted(page, "source:8");
    const camera = await read(page, ids.Ich, centres(8, 640, 480));
    assert.deepEqual(camera.size, [640, 480]);
    assert.equal(camera.fit, "cover");
    assert.ok(camera.mirrored, "the own camera is mirrored");
    BARS.forEach((colour, bar) => same(camera.pixels[bar], colour, "camera"));
    const live = await read(page, ids["Mein Live"], centres(8, 640, 360));
    assert.equal(live.fit, "contain");
    assert.ok(!live.mirrored, "the own Live is not mirrored");
    BARS.forEach((colour, bar) => same(live.pixels[bar], colour, "own Live"));
    const opened = await named(page, "media_view_open");
    assert.deepEqual(opened.slice(1), [
      { source: 7, maxWidth: 640, maxHeight: 360 },
      { source: 8, maxWidth: 640, maxHeight: 360 },
    ]);
    // The cropped camera tile needs the 4:3 picture a little larger.
    await page.waitForFunction(() =>
      window.smokeApp
        .named("media_view_configure")
        .some((args) => args.maxWidth === 640 && args.maxHeight === 480),
    );
    assert.equal(await page.getByText("Keine Vorschau").count(), 0);
    assert.equal(
      await page.getByRole("button", { name: "Eigenes Fenster" }).count(),
      1,
    );
    console.log("PASS: own camera mirrored and cropped, own Live uncropped");

    // The window stays an extra for other people's streams.
    await page.getByRole("button", { name: "Eigenes Fenster" }).click();
    await page.getByRole("button", { name: "Fenster schließen" }).waitFor();
    const [viewer] = await named(page, "media_viewer_open");
    assert.equal(viewer.consumer, 1);
    assert.equal(viewer.title, "Alex – Gelabber");
    await bars(
      page,
      ids.Alex,
      remote,
      { width: 640, height: 360, colours: BARS },
      "next to a viewer window",
    );
    await page.getByRole("button", { name: "Fenster schließen" }).click();
    assert.deepEqual(await named(page, "media_viewer_close"), [
      { consumer: 1 },
    ]);
    console.log("PASS: viewer window as a second way to watch");

    // The large view shows the same stream on a second canvas.
    await page
      .locator('[data-tile="Alex"]')
      .getByRole("button", { name: "Vergrößern" })
      .click();
    await page.locator("dialog canvas").waitFor();
    const large = await read(page, ids.Alex, centres(8, 640, 360), 1);
    assert.ok(large.shown, "the large view starts with the tile's picture");
    BARS.forEach((colour, bar) =>
      same(large.pixels[bar], colour, "large view"),
    );
    // The app is asked for more, but never for more than 1080p's pixels.
    const sizeOf = () =>
      page.evaluate(() =>
        window.smokeApp
          .named("media_view_configure")
          .filter((args) => args.view === 1)
          .at(-1),
      );
    await page.waitForFunction(() =>
      window.smokeApp
        .named("media_view_configure")
        .some((args) => args.view === 1 && args.maxWidth > 960),
    );
    const grown = await sizeOf();
    assert.ok(grown.maxWidth > 960 && grown.maxHeight > 540);
    assert.ok(grown.maxWidth * grown.maxHeight <= 1920 * 1080);
    assert.ok(
      (await page.evaluate(
        (id) => window.smoke.renderedVideoHeight(id),
        ids.Alex,
      )) > 540,
    );
    assert.equal(
      (await named(page, "media_view_open")).filter((args) => args.consumer)
        .length,
      1,
    );
    await bars(
      page,
      ids.Alex,
      remote,
      { width: 1280, height: 720, colours: BARS, bt709: true },
      "tile under the large view",
    );
    const both = await read(page, ids.Alex, centres(8, 1280, 720), 1);
    BARS.forEach((colour, bar) => same(both.pixels[bar], colour, "large view"));
    await page
      .locator("dialog")
      .getByRole("button", { name: "Schließen" })
      .click();
    await page.waitForFunction(() => {
      const last = window.smokeApp
        .named("media_view_configure")
        .filter((args) => args.view === 1)
        .at(-1);
      return last.maxWidth === 960 && last.maxHeight === 540;
    });
    console.log("PASS: tile and large view share one view, size follows");

    // Room focus mounts the tile somewhere else: same view, same picture.
    const calls = async () => [
      (await named(page, "media_view_open")).length,
      (await named(page, "media_view_close")).length,
    ];
    const settled = await calls();
    const moved = await page.evaluate(
      async ([list, id]) => {
        const before = document.querySelector(
          `canvas[data-native-track="${id}"]`,
        );
        window.smoke.show(list);
        // The first frame the browser paints after the move.
        await new Promise(requestAnimationFrame);
        const after = document.querySelector(
          `canvas[data-native-track="${id}"]`,
        );
        return {
          remounted: after !== before,
          size: [after.width, after.height],
          shown: getComputedStyle(after).opacity === "1",
        };
      },
      [
        tiles.map((tile) => ({ ...tile, focus: tile.name === "Alex" })),
        ids.Alex,
      ],
    );
    assert.deepEqual(moved, {
      remounted: true,
      size: [1280, 720],
      shown: true,
    });
    const kept = await read(page, ids.Alex, centres(8, 1280, 720));
    BARS.forEach((colour, bar) => same(kept.pixels[bar], colour, "moved tile"));
    await page.waitForTimeout(600);
    assert.deepEqual(await calls(), settled);
    await bars(
      page,
      ids.Alex,
      remote,
      { width: 640, height: 360, colours: BARS },
      "moved tile, next frame",
    );
    console.log("PASS: a tile that moves keeps its view and its picture");

    // More tiles than a page may have WebGL contexts.
    const many = Array.from({ length: 20 }, (_, tile) => ({
      name: `Gast ${tile}`,
      consumer: 100 + tile,
      width: 160,
    }));
    const colourOf = (tile) => [tile * 12, 255 - tile * 12, (tile * 53) % 256];
    for (const [tile, { consumer }] of many.entries())
      await push(page, `consumer:${consumer}`, {
        width: 320,
        height: 180,
        colours: [colourOf(tile)],
      });
    Object.assign(ids, await show(page, [...tiles, ...many]));
    for (const { consumer } of many)
      await painted(page, `consumer:${consumer}`);
    for (const [tile, { name }] of many.entries()) {
      const got = await read(page, ids[name], [[160, 90]]);
      assert.deepEqual(got.size, [320, 180]);
      same(got.pixels[0], colourOf(tile), name);
    }
    assert.equal(await page.evaluate(() => window.smokeApp.contexts.length), 1);
    console.log("PASS: 23 tiles drawn through one WebGL context");

    // Pictures of four sizes in one refresh, small and large mixed as the
    // tiles are.
    const converter = () =>
      page.evaluate(() => {
        const [{ canvas }] = window.smokeApp.contexts;
        return [canvas.width, canvas.height];
      });
    const stage = { width: 1920, height: 1080, colours: BARS, bt709: true };
    const selfView = { width: 640, height: 480, colours: [...BARS].reverse() };
    const ownLive = { width: 1280, height: 720, colours: [BLUE, RED] };
    const guest = (tile) => ({
      width: 320,
      height: 180,
      colours: [colourOf(19 - tile), WHITE],
    });
    await page.evaluate(
      (pushes) => {
        for (const [target, spec] of pushes) window.smokeApp.push(target, spec);
      },
      [
        ...many
          .slice(0, 10)
          .map(({ consumer }, tile) => [`consumer:${consumer}`, guest(tile)]),
        [remote, stage],
        ["source:7", selfView],
        ...many
          .slice(10)
          .map(({ consumer }, tile) => [
            `consumer:${consumer}`,
            guest(tile + 10),
          ]),
        ["source:8", ownLive],
      ],
    );
    for (const target of [remote, "source:7", "source:8"])
      await painted(page, target);
    for (const { consumer } of many)
      await painted(page, `consumer:${consumer}`);
    for (const [name, spec] of [
      ["Alex", stage],
      ["Ich", selfView],
      ["Mein Live", ownLive],
      ...many.map(({ name }, tile) => [name, guest(tile)]),
    ]) {
      const { width, height, colours } = spec;
      const got = await read(
        page,
        ids[name],
        centres(colours.length, width, height),
      );
      assert.deepEqual(got.size, [width, height], `${name}: bitmap size`);
      colours.forEach((colour, bar) =>
        same(got.pixels[bar], colour, `${name} among other sizes, bar ${bar}`),
      );
    }
    // In WebKitGTK copying a tile's picture costs by the canvas it is
    // converted on, not by the picture: there a camera must not go on
    // paying for a stage. Chromium keeps its canvas at the largest size.
    await bars(
      page,
      ids["Gast 7"],
      "consumer:107",
      { width: 320, height: 180, colours: BARS },
      "camera after the stage",
    );
    assert.deepEqual(
      await converter(),
      engine.canvasGrows ? [1920, 1080] : [320, 180],
    );
    await bars(page, ids.Alex, remote, stage, "stage after the camera");
    assert.deepEqual(await converter(), [1920, 1080]);
    assert.equal(await page.evaluate(() => window.smokeApp.contexts.length), 1);
    console.log("PASS: pictures of four sizes mixed in one refresh");

    // A lost context: nothing is drawn until it is back, then the newest.
    await page.evaluate(() => {
      const [gl] = window.smokeApp.contexts;
      for (const state of ["lost", "restored"])
        gl.canvas.addEventListener(
          `webglcontext${state}`,
          () => (window.smokeApp.context = state),
        );
      // A lost context hands out no extensions: keep this one.
      window.smokeApp.lose = gl.getExtension("WEBGL_lose_context");
      window.smokeApp.lose.loseContext();
    });
    await page.waitForFunction(() => window.smokeApp.context === "lost");
    await push(page, remote, { width: 320, height: 180, colours: [BLUE] });
    await page.waitForFunction((to) => window.smokeApp.caughtUp(to), remote);
    await frames(page);
    const stale = await read(page, ids.Alex, [[160, 90]]);
    assert.deepEqual(
      stale.size,
      [1920, 1080],
      "the picture from before the loss",
    );
    // Only a context whose loss the page handled can come back.
    await page.evaluate(() => window.smokeApp.lose.restoreContext());
    await page.waitForFunction(() => window.smokeApp.context === "restored");
    await frames(page);
    const back = await read(page, ids.Alex, [[160, 90]]);
    assert.deepEqual(back.size, [320, 180]);
    same(back.pixels[0], BLUE, "after the context came back");
    await bars(
      page,
      ids["Gast 3"],
      "consumer:103",
      { width: 320, height: 180, colours: BARS },
      "another tile after the context came back",
    );
    console.log("PASS: a lost WebGL context is survived");

    // Tiles gone: every view closes, nothing keeps asking for frames.
    await show(page, []);
    await page.waitForFunction(
      () =>
        window.smokeApp.named("media_view_close").length ===
        window.smokeApp.named("media_view_open").length,
    );
    assert.equal((await named(page, "media_view_open")).length, 23);
    const before = (await named(page, "media_view_frame")).length;
    await frames(page, 30);
    assert.equal((await named(page, "media_view_frame")).length, before);
    assert.deepEqual(await page.evaluate(() => window.smokeApp.violations), []);
    assert.deepEqual(problems, []);
    assert.deepEqual(warnings, []);
    assert.equal(
      await page.evaluate(() => window.smoke.nativeVideoLimit()),
      null,
    );
    console.log("PASS: views close with their tiles, one request at a time");
    await context.close();
  }

  // Tauri's IPC fetch blocked (a CSP on the server): frames arrive as JSON.
  {
    const { page, context, problems, warnings } = await open({
      features: ["video-frames"],
      json: true,
    });
    const ids = await show(page, [
      { name: "Alex", consumer: 1, width: 480, screen: true },
    ]);
    await bars(
      page,
      ids.Alex,
      "consumer:1",
      { width: 640, height: 360, colours: BARS },
      "JSON frames",
    );
    await page.getByText("Geringe Bildqualität").waitFor();
    assert.equal(
      await page.evaluate(() => window.smoke.nativeVideoLimit()),
      "transport",
    );
    assert.deepEqual((await named(page, "media_view_configure"))[0], {
      view: 1,
      maxWidth: 320,
      maxHeight: 180,
      maxFps: 15,
    });
    await bars(
      page,
      ids.Alex,
      "consumer:1",
      { width: 320, height: 180, colours: BARS },
      "small JSON frames",
    );
    assert.equal(
      await page.evaluate(
        (id) => window.smoke.renderedVideoHeight(id),
        ids.Alex,
      ),
      180,
    );
    // Views that open later are small from their first frame on.
    Object.assign(
      ids,
      await show(page, [
        { name: "Alex", consumer: 1, width: 480, screen: true },
        { name: "Ich", source: 7, width: 480, mirror: true },
      ]),
    );
    await page.waitForFunction(
      () => window.smokeApp.named("media_view_open").length === 2,
    );
    assert.deepEqual((await named(page, "media_view_open"))[1], {
      source: 7,
      maxWidth: 320,
      maxHeight: 180,
      maxFps: 15,
    });
    assert.equal(warnings.length, 1, "one warning for the whole page");
    assert.match(warnings[0], /connect-src/);
    assert.deepEqual(problems, []);
    console.log("PASS: JSON fallback detected, video kept small, hint shown");
    await context.close();
  }

  // No WebGL in the page: converted in script, and kept small.
  {
    const { page, context, problems, warnings } = await open({
      features: ["video-frames"],
      noWebgl: true,
    });
    const ids = await show(page, [
      { name: "Alex", consumer: 1, width: 480, screen: true },
    ]);
    assert.deepEqual(await named(page, "media_view_open"), [
      { consumer: 1, maxWidth: 320, maxHeight: 180, maxFps: 15 },
    ]);
    await bars(
      page,
      ids.Alex,
      "consumer:1",
      { width: 320, height: 180, colours: BARS },
      "BT.601 without WebGL",
    );
    await bars(
      page,
      ids.Alex,
      "consumer:1",
      { width: 321, height: 181, colours: BARS, bt709: true },
      "BT.709 without WebGL",
    );
    await turned(
      page,
      ids.Alex,
      "consumer:1",
      1,
      [BLUE, RED, WHITE, GREEN],
      "90°",
    );
    await turned(
      page,
      ids.Alex,
      "consumer:1",
      3,
      [GREEN, WHITE, RED, BLUE],
      "270°",
    );
    await page.getByText("Geringe Bildqualität").waitFor();
    assert.equal(
      await page.evaluate(() => window.smoke.nativeVideoLimit()),
      "renderer",
    );
    assert.equal(warnings.length, 1);
    assert.deepEqual(problems, []);
    console.log("PASS: pictures without WebGL, kept small, hint shown");
    await context.close();
  }

  // A released 0.5.x app: no features, so nothing changes for it.
  {
    const { page, context, problems, warnings } = await open({});
    await show(page, [
      { name: "Alex", consumer: 1, width: 480, screen: true },
      { name: "Ich", source: 7, width: 320, mirror: true },
    ]);
    await page.getByRole("button", { name: "Im Fenster ansehen" }).click();
    await page.getByRole("button", { name: "Fenster schließen" }).waitFor();
    assert.equal(
      await page.getByText("Keine Vorschau in der Desktop-App.").count(),
      1,
    );
    assert.equal(await page.locator("canvas").count(), 0);
    assert.deepEqual(
      await page.evaluate(() =>
        window.smokeApp.log.map((call) => call.command),
      ),
      ["media_info", "media_viewer_open"],
    );
    assert.deepEqual(problems, []);
    assert.deepEqual(warnings, []);
    console.log("PASS: a 0.5.x app keeps its viewer window and placeholder");
    await context.close();
  }

  console.log(
    `PASS: largest colour error ${worst} of 255 (limit ${TOLERANCE}), ${browser.browserType().name()} ${browser.version()}`,
  );
} finally {
  await browser.close();
  await rm(out, { recursive: true, force: true });
}
