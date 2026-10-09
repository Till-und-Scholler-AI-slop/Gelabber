// Video frames of the desktop app's native core, drawn in the page where the
// browser shows its <video>. The app hands them out as packets (I420 planes
// behind a small header, `media_view_frame`); WebGL turns them into RGB.
//
// WebKitGTK keeps only 16 WebGL contexts per page, so there is ONE context
// for every tile: a frame is converted on a canvas that is not in the page
// and copied from there onto the tile's 2D canvas.

export const FRAME_HEADER_LENGTH = 32;
const MAGIC = 0x31524647; // "GFR1", little endian
const MAX_SIDE = 16384;

export type Frame = {
  /** Size of the planes as sent. */
  width: number;
  height: number;
  chromaWidth: number;
  chromaHeight: number;
  /** Size of the picture once `rotation` is applied. */
  displayWidth: number;
  displayHeight: number;
  /** BT.709, otherwise BT.601; limited range either way. */
  bt709: boolean;
  /** Clockwise degrees the picture is turned by for display. */
  rotation: 0 | 90 | 180 | 270;
  /** Sequence number within the view, from 1. */
  seq: number;
  timestampUs: number;
  y: Uint8Array;
  u: Uint8Array;
  v: Uint8Array;
};

/** Splits a packet into its header fields and plane views (no copy). */
export function parsePacket(buffer: ArrayBuffer): Frame {
  if (buffer.byteLength < FRAME_HEADER_LENGTH)
    throw new Error("short frame packet");
  const header = new DataView(buffer, 0, FRAME_HEADER_LENGTH);
  if (header.getUint32(0, true) !== MAGIC)
    throw new Error("not a frame packet");
  // A later app may send a longer header; the planes follow it.
  const offset = header.getUint16(4, true);
  if (offset < FRAME_HEADER_LENGTH) throw new Error("bad frame header");
  const format = header.getUint8(6);
  if (format !== 0) throw new Error(`unknown pixel format ${format}`);
  const flags = header.getUint8(7);
  const width = header.getUint32(8, true);
  const height = header.getUint32(12, true);
  if (width < 1 || height < 1 || width > MAX_SIDE || height > MAX_SIDE)
    throw new Error(`bad frame size ${width}x${height}`);
  const chromaWidth = (width + 1) >> 1;
  const chromaHeight = (height + 1) >> 1;
  const luma = width * height;
  const chroma = chromaWidth * chromaHeight;
  if (buffer.byteLength < offset + luma + 2 * chroma)
    throw new Error("short frame packet");
  const rotation = (((flags >> 1) & 3) * 90) as Frame["rotation"];
  const turned = rotation === 90 || rotation === 270;
  return {
    width,
    height,
    chromaWidth,
    chromaHeight,
    displayWidth: turned ? height : width,
    displayHeight: turned ? width : height,
    bt709: (flags & 1) === 1,
    rotation,
    seq: header.getUint32(16, true),
    // Signed 64 bit; exact up to 2^53.
    timestampUs:
      header.getInt32(28, true) * 0x1_0000_0000 + header.getUint32(24, true),
    y: new Uint8Array(buffer, offset, luma),
    u: new Uint8Array(buffer, offset + luma, chroma),
    v: new Uint8Array(buffer, offset + luma + chroma, chroma),
  };
}

// Limited-range YCbCr to RGB, the same numbers as the viewer window
// (desktop/app/src/viewer.rs): Cr to R, Cb to G, Cr to G, Cb to B.
const BT601 = [1.402, 0.3441, 0.7141, 1.772] as const;
const BT709 = [1.5748, 0.1873, 0.4681, 1.8556] as const;
const LUMA_SCALE = 255 / 219;
const CHROMA_SCALE = 255 / 224;

/** The conversion without a GPU, for pages that have no WebGL: `out` takes
 * the picture as RGBA, `displayWidth * displayHeight * 4` bytes. */
export function convertFrame(frame: Frame, out: Uint8ClampedArray): void {
  const { width, height, chromaWidth, rotation, y, u, v } = frame;
  const matrix = frame.bt709 ? BT709 : BT601;
  const columns = frame.displayWidth;
  const rows = frame.displayHeight;
  let index = 0;
  for (let row = 0; row < rows; row++) {
    for (let column = 0; column < columns; column++) {
      let x = column;
      let line = row;
      if (rotation === 90) {
        x = row;
        line = height - 1 - column;
      } else if (rotation === 180) {
        x = width - 1 - column;
        line = height - 1 - row;
      } else if (rotation === 270) {
        x = width - 1 - row;
        line = column;
      }
      const luma = (y[line * width + x] - 16) * LUMA_SCALE;
      const chroma = (line >> 1) * chromaWidth + (x >> 1);
      const cb = (u[chroma] - 128) * CHROMA_SCALE;
      const cr = (v[chroma] - 128) * CHROMA_SCALE;
      out[index] = luma + matrix[0] * cr;
      out[index + 1] = luma - matrix[1] * cb - matrix[2] * cr;
      out[index + 2] = luma + matrix[3] * cb;
      out[index + 3] = 255;
      index += 4;
    }
  }
}

// `turn` and `shift` map a point of the picture to the frame's planes, which
// is where the rotation happens.
const VERTEX = `
attribute vec2 position;
uniform vec4 turn;
uniform vec2 shift;
varying vec2 uv;
void main() {
  vec2 picture = vec2((position.x + 1.0) * 0.5, (1.0 - position.y) * 0.5);
  uv = vec2(dot(turn.xy, picture), dot(turn.zw, picture)) + shift;
  gl_Position = vec4(position, 0.0, 1.0);
}`;

const FRAGMENT = `
#ifdef GL_FRAGMENT_PRECISION_HIGH
precision highp float;
#else
precision mediump float;
#endif
varying vec2 uv;
uniform sampler2D planeY;
uniform sampler2D planeU;
uniform sampler2D planeV;
uniform vec4 matrix;
void main() {
  float y = (texture2D(planeY, uv).r - 16.0 / 255.0) * (255.0 / 219.0);
  float u = (texture2D(planeU, uv).r - 128.0 / 255.0) * (255.0 / 224.0);
  float v = (texture2D(planeV, uv).r - 128.0 / 255.0) * (255.0 / 224.0);
  vec3 rgb = vec3(
    y + matrix.x * v,
    y - matrix.y * u - matrix.z * v,
    y + matrix.w * u);
  gl_FragColor = vec4(clamp(rgb, 0.0, 1.0), 1.0);
}`;

const PLANES = ["planeY", "planeU", "planeV"] as const;
/** The shared canvas grows in steps, so nearby sizes do not reallocate it. */
const CANVAS_STEP = 256;

type GL = WebGLRenderingContext | WebGL2RenderingContext;

/** The textures of one stream on the shared context. */
export type FrameSurface = {
  textures: WebGLTexture[] | null;
  width: number;
  height: number;
  /** The frame the textures hold. */
  loaded: Frame | null;
  /** The context generation the textures belong to. */
  epoch: number;
};

/** Converts frames on one WebGL context. The picture lands in the top left
 * corner of `canvas`, `displayWidth` x `displayHeight` pixels, until the
 * next `draw`. */
export class FrameRenderer {
  readonly canvas: HTMLCanvasElement;
  /** Runs when a lost context is back and pictures can be drawn again. */
  onRestored: (() => void) | null = null;
  private readonly gl: GL;
  private readonly webgl2: boolean;
  private lost = false;
  private epoch = 0;
  private turn: WebGLUniformLocation | null = null;
  private shift: WebGLUniformLocation | null = null;
  private matrix: WebGLUniformLocation | null = null;
  private readonly handleLost = (event: Event) => {
    // Without this the context stays lost.
    event.preventDefault();
    this.lost = true;
  };
  private readonly handleRestored = () => {
    try {
      this.setup();
    } catch {
      return;
    }
    this.lost = false;
    this.onRestored?.();
  };

  /** Throws when the page has no WebGL. */
  constructor(canvas: HTMLCanvasElement) {
    this.canvas = canvas;
    const attributes: WebGLContextAttributes = {
      alpha: false,
      antialias: false,
      depth: false,
      stencil: false,
      preserveDrawingBuffer: false,
      powerPreference: "high-performance",
    };
    const gl =
      (canvas.getContext("webgl2", attributes) as GL | null) ??
      (canvas.getContext("webgl", attributes) as GL | null);
    if (!gl) throw new Error("WebGL is not available");
    this.gl = gl;
    this.webgl2 =
      typeof WebGL2RenderingContext !== "undefined" &&
      gl instanceof WebGL2RenderingContext;
    canvas.addEventListener("webglcontextlost", this.handleLost);
    canvas.addEventListener("webglcontextrestored", this.handleRestored);
    this.setup();
  }

  /** Program and geometry; again after a restored context, whose textures
   * are gone (surfaces notice by `epoch`). */
  private setup(): void {
    const gl = this.gl;
    const compile = (type: number, source: string) => {
      const shader = gl.createShader(type);
      if (!shader) throw new Error("shader");
      gl.shaderSource(shader, source);
      gl.compileShader(shader);
      if (!gl.getShaderParameter(shader, gl.COMPILE_STATUS))
        throw new Error(gl.getShaderInfoLog(shader) ?? "shader");
      return shader;
    };
    const program = gl.createProgram();
    if (!program) throw new Error("program");
    gl.attachShader(program, compile(gl.VERTEX_SHADER, VERTEX));
    gl.attachShader(program, compile(gl.FRAGMENT_SHADER, FRAGMENT));
    gl.bindAttribLocation(program, 0, "position");
    gl.linkProgram(program);
    if (!gl.getProgramParameter(program, gl.LINK_STATUS))
      throw new Error(gl.getProgramInfoLog(program) ?? "program");
    gl.useProgram(program);
    // One triangle covering the viewport.
    gl.bindBuffer(gl.ARRAY_BUFFER, gl.createBuffer());
    gl.bufferData(
      gl.ARRAY_BUFFER,
      new Float32Array([-1, -1, 3, -1, -1, 3]),
      gl.STATIC_DRAW,
    );
    gl.enableVertexAttribArray(0);
    gl.vertexAttribPointer(0, 2, gl.FLOAT, false, 0, 0);
    // Planes are tightly packed, also at odd widths.
    gl.pixelStorei(gl.UNPACK_ALIGNMENT, 1);
    PLANES.forEach((name, unit) =>
      gl.uniform1i(gl.getUniformLocation(program, name), unit),
    );
    this.turn = gl.getUniformLocation(program, "turn");
    this.shift = gl.getUniformLocation(program, "shift");
    this.matrix = gl.getUniformLocation(program, "matrix");
    this.epoch++;
  }

  surface(): FrameSurface {
    return { textures: null, width: 0, height: 0, loaded: null, epoch: 0 };
  }

  /** Frees a surface's textures. */
  release(surface: FrameSurface): void {
    if (surface.textures && surface.epoch === this.epoch && !this.lost)
      for (const texture of surface.textures) this.gl.deleteTexture(texture);
    surface.textures = null;
    surface.loaded = null;
  }

  /** Gives the canvas's memory back while nothing is shown. */
  shrink(): void {
    this.canvas.width = 1;
    this.canvas.height = 1;
  }

  /** Uploads the frame's planes to its surface and draws the picture;
   * false while the context is lost. */
  draw(surface: FrameSurface, frame: Frame): boolean {
    if (this.lost) return false;
    const gl = this.gl;
    if (surface.epoch !== this.epoch) {
      surface.textures = null;
      surface.loaded = null;
      surface.epoch = this.epoch;
    }
    if (!surface.textures) {
      surface.textures = PLANES.map(() => {
        const texture = gl.createTexture();
        gl.bindTexture(gl.TEXTURE_2D, texture);
        gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.LINEAR);
        gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.LINEAR);
        gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_S, gl.CLAMP_TO_EDGE);
        gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_T, gl.CLAMP_TO_EDGE);
        return texture;
      });
      surface.width = 0;
      surface.height = 0;
    }
    const resized =
      frame.width !== surface.width || frame.height !== surface.height;
    const fresh = resized || surface.loaded !== frame;
    for (let unit = 0; unit < 3; unit++) {
      gl.activeTexture(gl.TEXTURE0 + unit);
      gl.bindTexture(gl.TEXTURE_2D, surface.textures[unit]);
      if (!fresh) continue;
      const data = unit === 0 ? frame.y : unit === 1 ? frame.u : frame.v;
      const width = unit === 0 ? frame.width : frame.chromaWidth;
      const height = unit === 0 ? frame.height : frame.chromaHeight;
      this.upload(data, width, height, resized);
    }
    surface.width = frame.width;
    surface.height = frame.height;
    surface.loaded = frame;

    const { displayWidth, displayHeight } = frame;
    const canvas = this.canvas;
    // Resizing clears and reallocates, so it only ever grows.
    if (canvas.width < displayWidth)
      canvas.width = Math.ceil(displayWidth / CANVAS_STEP) * CANVAS_STEP;
    if (canvas.height < displayHeight)
      canvas.height = Math.ceil(displayHeight / CANVAS_STEP) * CANVAS_STEP;
    // WebGL counts rows from the bottom: this is the canvas's top left.
    gl.viewport(0, canvas.height - displayHeight, displayWidth, displayHeight);
    if (frame.rotation === 90) {
      gl.uniform4f(this.turn, 0, 1, -1, 0);
      gl.uniform2f(this.shift, 0, 1);
    } else if (frame.rotation === 180) {
      gl.uniform4f(this.turn, -1, 0, 0, -1);
      gl.uniform2f(this.shift, 1, 1);
    } else if (frame.rotation === 270) {
      gl.uniform4f(this.turn, 0, -1, 1, 0);
      gl.uniform2f(this.shift, 1, 0);
    } else {
      gl.uniform4f(this.turn, 1, 0, 0, 1);
      gl.uniform2f(this.shift, 0, 0);
    }
    const matrix = frame.bt709 ? BT709 : BT601;
    gl.uniform4f(this.matrix, matrix[0], matrix[1], matrix[2], matrix[3]);
    gl.drawArrays(gl.TRIANGLES, 0, 3);
    return true;
  }

  private upload(
    data: Uint8Array,
    width: number,
    height: number,
    resized: boolean,
  ): void {
    const gl = this.gl;
    const format = this.webgl2
      ? (gl as WebGL2RenderingContext).RED
      : gl.LUMINANCE;
    if (resized)
      gl.texImage2D(
        gl.TEXTURE_2D,
        0,
        this.webgl2 ? (gl as WebGL2RenderingContext).R8 : gl.LUMINANCE,
        width,
        height,
        0,
        format,
        gl.UNSIGNED_BYTE,
        data,
      );
    else
      gl.texSubImage2D(
        gl.TEXTURE_2D,
        0,
        0,
        0,
        width,
        height,
        format,
        gl.UNSIGNED_BYTE,
        data,
      );
  }

  dispose(): void {
    this.canvas.removeEventListener("webglcontextlost", this.handleLost);
    this.canvas.removeEventListener(
      "webglcontextrestored",
      this.handleRestored,
    );
    this.gl.getExtension("WEBGL_lose_context")?.loseContext();
  }
}

/** Puts the frames of one stream on the canvases that show it. */
export type FramePainter = {
  /** Makes `frame` the picture `show` copies; false when it cannot be drawn
   * right now (lost context). */
  load(frame: Frame): boolean;
  /** Copies the loaded picture onto a tile's canvas, whose bitmap takes the
   * picture's size: CSS (`object-fit`, `transform`) fits, crops and mirrors
   * it like a <video>. */
  show(canvas: HTMLCanvasElement): void;
  dispose(): void;
};

const contexts = new WeakMap<
  HTMLCanvasElement,
  CanvasRenderingContext2D | null
>();

function context2d(
  canvas: HTMLCanvasElement,
  width: number,
  height: number,
): CanvasRenderingContext2D | null {
  let context = contexts.get(canvas);
  if (context === undefined) {
    context = canvas.getContext("2d", { alpha: false });
    contexts.set(canvas, context);
  }
  if (canvas.width !== width || canvas.height !== height) {
    canvas.width = width;
    canvas.height = height;
  }
  return context;
}

let shared: FrameRenderer | null | undefined;
const restored = new Set<() => void>();
let painters = 0;

function sharedRenderer(): FrameRenderer | null {
  if (shared === undefined) {
    try {
      shared = new FrameRenderer(document.createElement("canvas"));
      shared.onRestored = () => {
        for (const listener of [...restored]) listener();
      };
    } catch {
      shared = null;
    }
  }
  return shared;
}

/** A painter for one stream. `software` is true when the page has no WebGL
 * and the pictures are converted in script, which only small ones allow.
 * `onRestored` runs when pictures can be drawn again after a lost context. */
export function createFramePainter(onRestored: () => void): {
  painter: FramePainter;
  software: boolean;
} {
  const renderer = sharedRenderer();
  if (!renderer) return { painter: softwarePainter(), software: true };
  const surface = renderer.surface();
  let width = 0;
  let height = 0;
  restored.add(onRestored);
  painters++;
  return {
    software: false,
    painter: {
      load(frame) {
        if (!renderer.draw(surface, frame)) return false;
        width = frame.displayWidth;
        height = frame.displayHeight;
        return true;
      },
      show(canvas) {
        context2d(canvas, width, height)?.drawImage(
          renderer.canvas,
          0,
          0,
          width,
          height,
          0,
          0,
          width,
          height,
        );
      },
      dispose() {
        if (!restored.delete(onRestored)) return;
        renderer.release(surface);
        if (--painters === 0) renderer.shrink();
      },
    },
  };
}

function softwarePainter(): FramePainter {
  let image: ImageData | null = null;
  return {
    load(frame) {
      if (
        image?.width !== frame.displayWidth ||
        image.height !== frame.displayHeight
      )
        image = new ImageData(frame.displayWidth, frame.displayHeight);
      convertFrame(frame, image.data);
      return true;
    },
    show(canvas) {
      if (image)
        context2d(canvas, image.width, image.height)?.putImageData(image, 0, 0);
    },
    dispose() {
      image = null;
    },
  };
}
