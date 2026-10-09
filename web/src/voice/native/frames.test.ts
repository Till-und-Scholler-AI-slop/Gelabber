import { describe, expect, it, vi } from "vitest";

import {
  FRAME_HEADER_LENGTH,
  FrameRenderer,
  convertFrame,
  parsePacket,
  type Frame,
} from "./frames.ts";

type PacketOptions = {
  width: number;
  height: number;
  flags?: number;
  seq?: number;
  header?: number;
  /** Plane bytes for a pixel; chroma is asked once per 2x2 block. */
  y?: (x: number, line: number) => number;
  u?: (x: number, line: number) => number;
  v?: (x: number, line: number) => number;
};

/** A packet as the app sends it. */
function packet(options: PacketOptions): ArrayBuffer {
  const { width, height } = options;
  const header = options.header ?? FRAME_HEADER_LENGTH;
  const chromaWidth = Math.ceil(width / 2);
  const chromaHeight = Math.ceil(height / 2);
  const bytes = new Uint8Array(
    header + width * height + 2 * chromaWidth * chromaHeight,
  );
  const view = new DataView(bytes.buffer);
  bytes.set([0x47, 0x46, 0x52, 0x31]); // "GFR1"
  view.setUint16(4, header, true);
  view.setUint8(6, 0);
  view.setUint8(7, options.flags ?? 0);
  view.setUint32(8, width, true);
  view.setUint32(12, height, true);
  view.setUint32(16, options.seq ?? 1, true);
  view.setBigInt64(24, 1_234_567n, true);
  let at = header;
  for (let line = 0; line < height; line++)
    for (let x = 0; x < width; x++) bytes[at++] = options.y?.(x, line) ?? 16;
  for (const plane of [options.u, options.v])
    for (let line = 0; line < chromaHeight; line++)
      for (let x = 0; x < chromaWidth; x++)
        bytes[at++] = plane?.(x, line) ?? 128;
  return bytes.buffer;
}

const frame = (options: PacketOptions) => parsePacket(packet(options));

describe("frame packets", () => {
  it("reads the header and the planes in place", () => {
    const buffer = packet({
      width: 6,
      height: 4,
      seq: 77,
      y: (x, line) => 10 * line + x,
      u: (x, line) => 100 + 10 * line + x,
      v: (x, line) => 200 + 10 * line + x,
    });
    const parsed = parsePacket(buffer);
    expect(parsed).toMatchObject({
      width: 6,
      height: 4,
      chromaWidth: 3,
      chromaHeight: 2,
      displayWidth: 6,
      displayHeight: 4,
      bt709: false,
      rotation: 0,
      seq: 77,
      timestampUs: 1_234_567,
    });
    expect(parsed.y.buffer).toBe(buffer);
    expect([parsed.y.byteOffset, parsed.y.length]).toEqual([32, 24]);
    expect([parsed.u.byteOffset, parsed.u.length]).toEqual([56, 6]);
    expect([parsed.v.byteOffset, parsed.v.length]).toEqual([62, 6]);
    expect(parsed.y[6 + 2]).toBe(12);
    expect([...parsed.u]).toEqual([100, 101, 102, 110, 111, 112]);
    expect(parsed.v[5]).toBe(212);
  });

  it("rounds the chroma planes of odd sizes up", () => {
    const parsed = frame({ width: 5, height: 3 });
    expect(parsed.chromaWidth).toBe(3);
    expect(parsed.chromaHeight).toBe(2);
    expect(parsed.y).toHaveLength(15);
    expect(parsed.u).toHaveLength(6);
    expect(parsed.v).toHaveLength(6);
    expect(frame({ width: 1, height: 1 }).u).toHaveLength(1);
    // One byte less than 5x3 needs.
    expect(() =>
      parsePacket(packet({ width: 5, height: 3 }).slice(0, 58)),
    ).toThrow("short frame packet");
  });

  it("reads matrix and rotation from the flags", () => {
    expect(frame({ width: 4, height: 2, flags: 0b001 })).toMatchObject({
      bt709: true,
      rotation: 0,
    });
    expect(frame({ width: 4, height: 2, flags: 0b010 })).toMatchObject({
      bt709: false,
      rotation: 90,
      displayWidth: 2,
      displayHeight: 4,
    });
    expect(frame({ width: 4, height: 2, flags: 0b101 })).toMatchObject({
      bt709: true,
      rotation: 180,
      displayWidth: 4,
      displayHeight: 2,
    });
    expect(frame({ width: 4, height: 2, flags: 0b110 })).toMatchObject({
      rotation: 270,
      displayWidth: 2,
      displayHeight: 4,
    });
  });

  it("reads timestamps beyond 32 bits and a longer header", () => {
    const buffer = packet({ width: 2, height: 2, header: 40, y: () => 99 });
    const view = new DataView(buffer);
    view.setBigInt64(24, 1_760_000_000_000_000n, true);
    const parsed = parsePacket(buffer);
    expect(parsed.timestampUs).toBe(1_760_000_000_000_000);
    expect(parsed.y.byteOffset).toBe(40);
    expect([...parsed.y]).toEqual([99, 99, 99, 99]);
    view.setBigInt64(24, -5n, true);
    expect(parsePacket(buffer).timestampUs).toBe(-5);
  });

  it("rejects what is not a complete packet", () => {
    const good = packet({ width: 4, height: 2 });
    const changed = (change: (view: DataView) => void) => {
      const copy = good.slice(0);
      change(new DataView(copy));
      return copy;
    };
    expect(() => parsePacket(new ArrayBuffer(0))).toThrow("short frame packet");
    expect(() => parsePacket(good.slice(0, 31))).toThrow("short frame packet");
    expect(() => parsePacket(good.slice(0, 32))).toThrow("short frame packet");
    expect(() =>
      parsePacket(changed((view) => view.setUint8(3, 0x32))),
    ).toThrow("not a frame packet");
    expect(() =>
      parsePacket(changed((view) => view.setUint16(4, 16, true))),
    ).toThrow("bad frame header");
    expect(() => parsePacket(changed((view) => view.setUint8(6, 1)))).toThrow(
      "unknown pixel format 1",
    );
    expect(() =>
      parsePacket(changed((view) => view.setUint32(8, 0, true))),
    ).toThrow("bad frame size");
    expect(() =>
      parsePacket(changed((view) => view.setUint32(12, 70_000, true))),
    ).toThrow("bad frame size");
    // A header that claims more picture than the packet carries.
    expect(() =>
      parsePacket(changed((view) => view.setUint32(8, 6, true))),
    ).toThrow("short frame packet");
  });
});

/** RGB to limited-range YCbCr, the inverse of what the page does. */
function encode(rgb: readonly number[], bt709: boolean): number[] {
  const [kr, kb] = bt709 ? [0.2126, 0.0722] : [0.299, 0.114];
  const [r, g, b] = rgb.map((value) => value / 255);
  const luma = kr * r + (1 - kr - kb) * g + kb * b;
  return [
    Math.round(16 + 219 * luma),
    Math.round(128 + (224 * (b - luma)) / (2 * (1 - kb))),
    Math.round(128 + (224 * (r - luma)) / (2 * (1 - kr))),
  ];
}

const BARS = [
  [255, 255, 255],
  [255, 255, 0],
  [0, 255, 255],
  [0, 255, 0],
  [255, 0, 255],
  [255, 0, 0],
  [0, 0, 255],
  [0, 0, 0],
  [128, 128, 128],
  [40, 90, 200],
] as const;

function pixels(parsed: Frame): number[][] {
  const out = new Uint8ClampedArray(
    parsed.displayWidth * parsed.displayHeight * 4,
  );
  convertFrame(parsed, out);
  return Array.from({ length: out.length / 4 }, (_, index) => [
    ...out.subarray(4 * index, 4 * index + 4),
  ]);
}

describe("frame conversion without a GPU", () => {
  it.each([
    ["BT.601", false],
    ["BT.709", true],
  ])("turns limited-range %s into RGB", (_, bt709) => {
    const coded = BARS.map((bar) => encode(bar, bt709));
    // Two pixels per bar, so every bar has chroma of its own.
    const rgba = pixels(
      frame({
        width: 2 * BARS.length,
        height: 2,
        flags: bt709 ? 1 : 0,
        y: (x) => coded[x >> 1][0],
        u: (x) => coded[x][1],
        v: (x) => coded[x][2],
      }),
    );
    BARS.forEach((bar, index) => {
      const [r, g, b, a] = rgba[2 * index];
      expect(a).toBe(255);
      for (const [got, want] of [
        [r, bar[0]],
        [g, bar[1]],
        [b, bar[2]],
      ])
        expect(Math.abs(got - want)).toBeLessThanOrEqual(2);
    });
  });

  it("uses the matrix the packet names", () => {
    // BT.709 red read as BT.601 comes out visibly darker.
    const [luma, cb, cr] = encode([255, 0, 0], true);
    const [wrong] = pixels(
      frame({ width: 2, height: 2, y: () => luma, u: () => cb, v: () => cr }),
    );
    expect(wrong[0]).toBeLessThan(240);
  });

  it("clamps what lies outside the video range", () => {
    const [low] = pixels(frame({ width: 2, height: 2, y: () => 0 }));
    const [high] = pixels(frame({ width: 2, height: 2, y: () => 255 }));
    expect(low).toEqual([0, 0, 0, 255]);
    expect(high).toEqual([255, 255, 255, 255]);
  });

  it("turns the picture clockwise by the rotation flag", () => {
    // Luma names the pixel: 16 + 20 * (line * 3 + x), on a 3x2 frame
    //   a b c
    //   d e f
    const turned = (flags: number) => {
      const parsed = frame({
        width: 3,
        height: 2,
        flags,
        y: (x, line) => 16 + 20 * (line * 3 + x),
      });
      const names = pixels(parsed).map(
        ([red]) => "abcdef"[Math.round((red * 219) / 255 / 20)],
      );
      const rows: string[] = [];
      for (let row = 0; row < parsed.displayHeight; row++)
        rows.push(
          names
            .slice(row * parsed.displayWidth, (row + 1) * parsed.displayWidth)
            .join(""),
        );
      return rows;
    };
    expect(turned(0b000)).toEqual(["abc", "def"]);
    expect(turned(0b010)).toEqual(["da", "eb", "fc"]);
    expect(turned(0b100)).toEqual(["fed", "cba"]);
    expect(turned(0b110)).toEqual(["cf", "be", "ad"]);
  });
});

type GlCall = [name: string, ...args: unknown[]];

/** A WebGL context that only records: constants are their names. */
function fakeContext() {
  const calls: GlCall[] = [];
  let textures = 0;
  const gl = new Proxy(
    {},
    {
      get(_, name: string) {
        if (name === "TEXTURE0") return 0;
        if (/^[A-Z][A-Z0-9_]*$/.test(name)) return name;
        return (...args: unknown[]) => {
          calls.push([name, ...args]);
          if (name === "createTexture") return { texture: ++textures };
          if (name === "getUniformLocation") return args[1];
          if (name === "getShaderParameter" || name === "getProgramParameter")
            return true;
          if (name.startsWith("create")) return {};
          return null;
        };
      },
    },
  );
  const listeners = new Map<string, (event: Event) => void>();
  const size = { width: 300, height: 150 };
  let resizes = 0;
  const canvas = {
    get width() {
      return size.width;
    },
    // Like the real one, every assignment allocates anew.
    set width(value: number) {
      size.width = value;
      resizes++;
    },
    get height() {
      return size.height;
    },
    set height(value: number) {
      size.height = value;
      resizes++;
    },
    getContext: (kind: string) => (kind === "webgl" ? gl : null),
    addEventListener: (type: string, listener: (event: Event) => void) =>
      listeners.set(type, listener),
    removeEventListener: (type: string) => listeners.delete(type),
  };
  return {
    canvas: canvas as unknown as HTMLCanvasElement,
    resizes: () => resizes,
    calls,
    named: (name: string) => calls.filter((call) => call[0] === name),
    emit: (type: string) => {
      const event = { preventDefault: vi.fn() };
      listeners.get(type)?.(event as unknown as Event);
      return event;
    },
  };
}

describe("frame renderer", () => {
  it("allocates the planes once per size and updates them after that", () => {
    const context = fakeContext();
    const renderer = new FrameRenderer(context.canvas);
    const surface = renderer.surface();
    const first = frame({ width: 6, height: 4 });
    expect(renderer.draw(surface, first)).toBe(true);
    expect(
      context.named("texImage2D").map((call) => [call[4], call[5]]),
    ).toEqual([
      [6, 4],
      [3, 2],
      [3, 2],
    ]);
    expect(context.named("texImage2D")[0][9]).toBe(first.y);
    expect(context.named("texSubImage2D")).toHaveLength(0);
    expect(context.named("drawArrays")).toHaveLength(1);

    const second = frame({ width: 6, height: 4, seq: 2 });
    renderer.draw(surface, second);
    expect(context.named("texImage2D")).toHaveLength(3);
    expect(
      context.named("texSubImage2D").map((call) => [call[5], call[6], call[9]]),
    ).toEqual([
      [6, 4, second.y],
      [3, 2, second.u],
      [3, 2, second.v],
    ]);
    // The same frame again (a canvas joined): nothing to upload.
    renderer.draw(surface, second);
    expect(context.named("texSubImage2D")).toHaveLength(3);
    expect(context.named("drawArrays")).toHaveLength(3);

    renderer.draw(surface, frame({ width: 8, height: 4, seq: 3 }));
    expect(context.named("texImage2D")).toHaveLength(6);
    expect(context.named("createTexture")).toHaveLength(3);
  });

  it("gives the canvas the size of the picture it converts", () => {
    const context = fakeContext();
    const renderer = new FrameRenderer(context.canvas);
    const stage = renderer.surface();
    const camera = renderer.surface();
    const other = renderer.surface();
    renderer.draw(stage, frame({ width: 1280, height: 720, flags: 1 }));
    expect([context.canvas.width, context.canvas.height]).toEqual([1280, 720]);
    expect(context.named("viewport").at(-1)).toEqual([
      "viewport",
      0,
      0,
      1280,
      720,
    ]);
    expect(context.named("uniform4f").at(-1)).toEqual([
      "uniform4f",
      "matrix",
      1.5748,
      0.1873,
      0.4681,
      1.8556,
    ]);
    // A smaller stream gets a smaller canvas: a copy from it costs by the
    // canvas, so a camera must not pay for the stage.
    renderer.draw(camera, frame({ width: 320, height: 180 }));
    expect([context.canvas.width, context.canvas.height]).toEqual([320, 180]);
    expect(context.named("viewport").at(-1)).toEqual([
      "viewport",
      0,
      0,
      320,
      180,
    ]);
    expect(context.named("uniform4f").at(-1)?.slice(2)).toEqual([
      1.402, 0.3441, 0.7141, 1.772,
    ]);
    expect(stage.textures).toHaveLength(3);
    expect(camera.textures).toHaveLength(3);
    expect(camera.textures![0]).not.toBe(stage.textures![0]);
    // Each stream's own textures are bound for its draw.
    const bound = context.named("bindTexture").slice(-3);
    expect(bound.map((call) => call[2])).toEqual(camera.textures);

    // Pictures of the same size leave the canvas alone.
    const resizes = context.resizes();
    renderer.draw(other, frame({ width: 320, height: 180 }));
    renderer.draw(camera, frame({ width: 320, height: 180, seq: 2 }));
    expect(context.resizes()).toBe(resizes);
    // A turned picture counts by what is shown.
    renderer.draw(other, frame({ width: 180, height: 320, flags: 0b010 }));
    expect(context.resizes()).toBe(resizes);
    renderer.draw(other, frame({ width: 320, height: 180, flags: 0b010 }));
    expect([context.canvas.width, context.canvas.height]).toEqual([180, 320]);

    renderer.release(camera);
    expect(context.named("deleteTexture")).toHaveLength(3);
    expect(camera.textures).toBeNull();
    renderer.shrink();
    expect([context.canvas.width, context.canvas.height]).toEqual([1, 1]);
  });

  it("keeps the largest size where a new one costs more than a large canvas", () => {
    const context = fakeContext();
    const renderer = new FrameRenderer(context.canvas, true);
    const stage = renderer.surface();
    const camera = renderer.surface();
    renderer.draw(stage, frame({ width: 1280, height: 720 }));
    expect([context.canvas.width, context.canvas.height]).toEqual([1280, 720]);
    const resizes = context.resizes();
    // The camera's picture is the canvas's top left; WebGL counts rows from
    // the bottom.
    renderer.draw(camera, frame({ width: 320, height: 180 }));
    renderer.draw(stage, frame({ width: 1280, height: 720, seq: 2 }));
    renderer.draw(camera, frame({ width: 320, height: 180, seq: 2 }));
    expect(context.resizes()).toBe(resizes);
    expect(context.named("viewport").at(-1)).toEqual([
      "viewport",
      0,
      540,
      320,
      180,
    ]);
    // A turned picture needs more height.
    renderer.draw(camera, frame({ width: 1000, height: 600, flags: 0b110 }));
    expect([context.canvas.width, context.canvas.height]).toEqual([1280, 1000]);
    expect(context.named("viewport").at(-1)).toEqual([
      "viewport",
      0,
      0,
      600,
      1000,
    ]);
    renderer.shrink();
    expect([context.canvas.width, context.canvas.height]).toEqual([1, 1]);
  });

  it("turns the picture in the shader", () => {
    const context = fakeContext();
    const renderer = new FrameRenderer(context.canvas);
    const turns = (flags: number) => {
      renderer.draw(
        renderer.surface(),
        frame({ width: 640, height: 360, flags }),
      );
      return [
        context.named("uniform4f").at(-2)?.slice(2),
        context.named("uniform2f").at(-1)?.slice(2),
        context.named("viewport").at(-1)?.slice(3),
      ];
    };
    expect(turns(0b000)).toEqual([
      [1, 0, 0, 1],
      [0, 0],
      [640, 360],
    ]);
    expect(turns(0b010)).toEqual([
      [0, 1, -1, 0],
      [0, 1],
      [360, 640],
    ]);
    expect(turns(0b100)).toEqual([
      [-1, 0, 0, -1],
      [1, 1],
      [640, 360],
    ]);
    expect(turns(0b110)).toEqual([
      [0, -1, 1, 0],
      [1, 0],
      [360, 640],
    ]);
  });

  it("waits out a lost context and starts over when it is back", () => {
    const context = fakeContext();
    const renderer = new FrameRenderer(context.canvas);
    const restored = vi.fn();
    renderer.onRestored = restored;
    const surface = renderer.surface();
    const picture = frame({ width: 6, height: 4 });
    renderer.draw(surface, picture);
    const before = context.calls.length;

    // The default action would keep the context lost for good.
    expect(context.emit("webglcontextlost").preventDefault).toHaveBeenCalled();
    expect(renderer.draw(surface, picture)).toBe(false);
    renderer.release(surface);
    expect(context.calls).toHaveLength(before);

    context.emit("webglcontextrestored");
    expect(restored).toHaveBeenCalledTimes(1);
    expect(context.named("linkProgram")).toHaveLength(2);
    // The old textures went with the old context.
    expect(renderer.draw(surface, picture)).toBe(true);
    expect(context.named("createTexture")).toHaveLength(6);
    expect(context.named("texImage2D")).toHaveLength(6);
    expect(context.named("deleteTexture")).toHaveLength(0);
  });

  it("says so when the page has no WebGL", () => {
    const canvas = { getContext: () => null } as unknown as HTMLCanvasElement;
    expect(() => new FrameRenderer(canvas)).toThrow("WebGL is not available");
  });
});
