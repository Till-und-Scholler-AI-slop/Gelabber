"""Stand-in Gelabber origin for the desktop app's smoke test.

Serves a page that calls the media commands through Tauri's IPC the way the
web client will, plus one command reserved for the bundled setup page, and
checks the report the page posts back. Exits 0 when the server origin reaches
the media core and nothing else, and video frames reach the page: views of a
test pattern (the app runs with GELABBER_VIDEO_TEST_PATTERN) deliver packets
with the right header and pixels.
"""

import json
import sys
import time
from http.server import BaseHTTPRequestHandler, HTTPServer

PAGE = b"""<!doctype html>
<html><head><meta charset="utf-8"><title>smoke</title></head><body>
<script>
(async () => {
  const ipc = window.__TAURI_INTERNALS__;
  const report = { ipc: Boolean(ipc) };
  const call = async (name, command, args) => {
    try { report[name] = { ok: await ipc.invoke(command, args) }; }
    catch (error) { report[name] = { error: String(error) }; }
  };
  if (ipc) {
    await call("info", "media_info");
    await call("devices", "media_audio_devices");
    await call("levels", "media_audio_levels");
    await call("setServer", "set_server", { server: "https://elsewhere.invalid" });
    report.video = await video(ipc).catch((error) => ({ failed: String(error) }));
  }
  await fetch("/report", { method: "POST", body: JSON.stringify(report) });
})();

// A frame packet as the page gets it: header fields and a few pixels.
function parse(body) {
  const bytes = body instanceof ArrayBuffer ? new Uint8Array(body) : Uint8Array.from(body);
  const header = new DataView(bytes.buffer, 0, 32);
  const width = header.getUint32(8, true);
  const height = header.getUint32(12, true);
  const chromaWidth = (width + 1) >> 1;
  const luma = width * height;
  const chroma = chromaWidth * ((height + 1) >> 1);
  const at = (x, y) => {
    const offset = (y >> 1) * chromaWidth + (x >> 1);
    return [bytes[32 + y * width + x], bytes[32 + luma + offset], bytes[32 + luma + chroma + offset]];
  };
  return {
    binary: body instanceof ArrayBuffer,
    length: bytes.length,
    magic: String.fromCharCode(...bytes.subarray(0, 4)),
    headerLength: header.getUint16(4, true),
    format: bytes[6],
    flags: bytes[7],
    width,
    height,
    seq: header.getUint32(16, true),
    reserved: header.getUint32(20, true),
    timestampUs: Number(header.getBigInt64(24, true)),
    // The middle of each of the eight colour bars, a quarter down.
    bars: [0, 1, 2, 3, 4, 5, 6, 7].map((bar) => at(Math.floor(((bar + 0.5) * width) / 8), height >> 2)),
  };
}

async function video(ipc) {
  const out = {};
  const frame = async (view, after) => parse(await ipc.invoke("media_view_frame", { view, after }));
  const error = (promise) => promise.then(() => null, (reason) => String(reason));
  const pattern = { width: 640, height: 360, fps: 30 };

  // A view smaller than the stream, then grown to its size.
  const { view } = await ipc.invoke("media_view_open", { testPattern: pattern, maxWidth: 320, maxHeight: 180 });
  out.first = await frame(view);
  out.second = await frame(view, out.first.seq);
  await ipc.invoke("media_view_configure", { view, maxWidth: 1881.6, maxHeight: 1058.4 });
  let last = out.second;
  for (let tries = 0; tries < 30 && last.width !== 640; tries++) last = await frame(view, last.seq);
  out.grown = last;

  // A second view of the same stream counts its own frames; one a second,
  // its request is still waiting when the view closes.
  const other = (await ipc.invoke("media_view_open", { testPattern: pattern })).view;
  out.otherFirst = await frame(other);
  await ipc.invoke("media_view_configure", { view: other, maxWidth: 640, maxHeight: 360, maxFps: 1 });
  const paced = await frame(other, out.otherFirst.seq);
  const waiting = error(ipc.invoke("media_view_frame", { view: other, after: paced.seq }));
  await ipc.invoke("media_view_close", { view: other });
  out.closedWhileWaiting = await waiting;
  out.afterClose = await error(ipc.invoke("media_view_frame", { view: other }));

  // The first view runs on: about a second of frames.
  const started = performance.now();
  out.frames = 0;
  while (performance.now() - started < 1000) {
    const next = await frame(view, last.seq);
    out.inOrder = (out.inOrder ?? true) && next.seq > last.seq && next.timestampUs > last.timestampUs;
    last = next;
    out.frames++;
  }
  await ipc.invoke("media_view_close", { view });
  out.closed = await error(ipc.invoke("media_view_frame", { view }));

  out.neither = await error(ipc.invoke("media_view_open", {}));
  out.both = await error(ipc.invoke("media_view_open", { consumer: 1, source: 1 }));
  out.unknownConsumer = await error(ipc.invoke("media_view_open", { consumer: 987654 }));
  out.unknownSource = await error(ipc.invoke("media_view_open", { source: 987654 }));
  out.unknownView = await error(ipc.invoke("media_view_configure", { view: 987654, maxWidth: 1, maxHeight: 1 }));
  return out;
}
</script></body></html>"""


def limited(rgb):
    """BT.601 limited-range Y, Cb, Cr of an RGB colour (0..1 per channel)."""
    luma = 0.299 * rgb[0] + 0.587 * rgb[1] + 0.114 * rgb[2]
    return [
        round(16 + 219 * luma),
        round(128 + 224 * (rgb[2] - luma) / 1.772),
        round(128 + 224 * (rgb[0] - luma) / 1.402),
    ]


# White, yellow, cyan, green, magenta, red, blue, black (frames.rs).
BARS = [limited(rgb) for rgb in [(1, 1, 1), (1, 1, 0), (0, 1, 1), (0, 1, 0), (1, 0, 1), (1, 0, 0), (0, 0, 1), (0, 0, 0)]]


def check_frame(name, frame, width, height, problems):
    if not isinstance(frame, dict):
        problems.append(f"{name}: no frame")
        return
    chroma = ((width + 1) // 2) * ((height + 1) // 2)
    expected = {
        "binary": True,
        "length": 32 + width * height + 2 * chroma,
        "magic": "GFR1",
        "headerLength": 32,
        "format": 0,
        # 360 lines: BT.601, no rotation.
        "flags": 0,
        "width": width,
        "height": height,
        "reserved": 0,
        "bars": BARS,
    }
    for key, value in expected.items():
        if frame.get(key) != value:
            problems.append(f"{name}: {key} is {frame.get(key)}, expected {value}")
    if abs(frame.get("timestampUs", 0) / 1e6 - time.time()) > 120:
        problems.append(f"{name}: timestamp {frame.get('timestampUs')} is not the time")


def check_video(video, problems):
    if "failed" in video or not video:
        problems.append(f"video views failed: {video.get('failed', 'no report')}")
        return
    check_frame("first frame", video.get("first"), 320, 180, problems)
    check_frame("second frame", video.get("second"), 320, 180, problems)
    check_frame("frame of the grown view", video.get("grown"), 640, 360, problems)
    check_frame("first frame of the second view", video.get("otherFirst"), 640, 360, problems)
    numbers = [video.get(name, {}).get("seq") for name in ("first", "second", "otherFirst")]
    if numbers[0] != 1 or not numbers[1] or numbers[1] <= 1 or numbers[2] != 1:
        problems.append(f"sequence numbers of the views are {numbers}, expected 1, more, 1")
    if not video.get("inOrder") or video.get("frames", 0) < 5:
        problems.append(f"{video.get('frames')} frames in a second, in order: {video.get('inOrder')}")
    for name in ("closedWhileWaiting", "afterClose", "closed", "neither", "both", "unknownConsumer",
                 "unknownSource", "unknownView"):
        if not video.get(name):
            problems.append(f"video: {name} did not fail")


class Handler(BaseHTTPRequestHandler):
    def do_GET(self):
        self.send_response(200)
        self.send_header("Content-Type", "text/html; charset=utf-8")
        self.end_headers()
        self.wfile.write(PAGE)

    def do_POST(self):
        length = int(self.headers.get("Content-Length", "0"))
        report = json.loads(self.rfile.read(length) or b"{}")
        self.send_response(204)
        self.end_headers()
        print("report:", json.dumps(report, indent=2), flush=True)
        problems = []
        if not report.get("ipc"):
            problems.append("no Tauri IPC on the server page")
        if report.get("info", {}).get("ok", {}).get("abi") != int(sys.argv[2]):
            problems.append("media_info did not answer with the core's ABI")
        if "inputs" not in report.get("devices", {}).get("ok", {}):
            problems.append("media_audio_devices failed")
        if "ok" not in report.get("levels", {}):
            problems.append("media_audio_levels failed")
        if "ok" in report.get("setServer", {}):
            problems.append("the server origin may call set_server")
        check_video(report.get("video", {}), problems)
        for problem in problems:
            print("FAIL:", problem, flush=True)
        self.server.result = 1 if problems else 0

    def log_message(self, *args):
        pass


server = HTTPServer(("127.0.0.1", int(sys.argv[1])), Handler)
server.result = None
server.timeout = 1
while server.result is None:
    server.handle_request()
sys.exit(server.result)
