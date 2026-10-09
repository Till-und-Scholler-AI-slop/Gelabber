"""Stand-in Gelabber origin for the desktop app's smoke test.

Serves a page that calls the media commands through Tauri's IPC the way the
web client will, plus one command reserved for the bundled setup page, and
checks the report the page posts back. Exits 0 when the server origin reaches
the media core and nothing else, and video frames reach the page: views of a
test pattern (the app runs with GELABBER_VIDEO_TEST_PATTERN) deliver packets
with the right header and pixels.

With a third argument, a file name (smoke.sh): the page's first report is not
the end. The page leaves a view open, this server creates the file, and the
caller ends the page's web process as a crash would. The app has to load the
page again by itself; that page reports as the first did and must not find
the view the first one left. Its request is answered only after HOLD_SECOND
seconds: until the new page commits, only the app's own answer to the ended
web process can have closed what the old page left (smoke.sh looks then).

On Windows the page also asks for screen capture and application sound, which
the Windows core does not have yet: the answers must be an empty application
list and a clean "not available on Windows yet" for both sources, as an older
server's web client, which asks without looking at the features, meets them.
On Linux they are left out: a screen capture would open the portal's picker.

Usage: smoke_server.py <port> <ABI version> [<file to create after the first report>]
"""

import json
import pathlib
import sys
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

# How long the app may take to bring the page back after its web process
# ended, report included.
BACK_WITHIN = 30

WINDOWS = sys.platform == "win32"

# How long the page's second request waits for its answer.
HOLD_SECOND = 2

# LEAVE and LEFT are filled in per page load (Handler.do_GET).
PAGE = b"""<!doctype html>
<html><head><meta charset="utf-8"><title>smoke</title></head><body>
<script>
// Whether this page leaves a view open for the page after it, and the view
// the page before it left; whether the app runs on Windows.
const LEAVE = __LEAVE__, LEFT = __LEFT__, WINDOWS = __WINDOWS__;
(async () => {
  const ipc = window.__TAURI_INTERNALS__;
  const report = { ipc: Boolean(ipc) };
  const call = async (name, command, args) => {
    try { report[name] = { ok: await ipc.invoke(command, args) }; }
    catch (error) { report[name] = { error: String(error) }; }
  };
  if (ipc) {
    if (LEFT !== null) {
      report.leftover = await ipc.invoke("media_view_frame", { view: LEFT })
        .then(() => "the view is still there", (error) => String(error));
    }
    await call("info", "media_info");
    await call("devices", "media_audio_devices");
    await call("levels", "media_audio_levels");
    await call("setServer", "set_server", { server: "https://elsewhere.invalid" });
    if (WINDOWS) {
      await call("audioApps", "media_audio_apps");
      await call("screen", "media_source_screen", { options: { type: "any", fps: 30, cursor: true, contentHint: "detail" } });
      await call("appAudio", "media_source_app_audio", { options: { app: "" } });
    }
    report.video = await video(ipc).catch((error) => ({ failed: String(error) }));
    if (LEAVE) {
      report.left = await ipc.invoke("media_view_open", { testPattern: { width: 640, height: 360, fps: 30 } })
        .then((opened) => opened.view, (error) => String(error));
    }
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

  // About a second of frames.
  const started = performance.now();
  out.frames = 0;
  while (performance.now() - started < 1000) {
    const next = await frame(view, last.seq);
    out.inOrder = (out.inOrder ?? true) && next.seq > last.seq && next.timestampUs > last.timestampUs;
    last = next;
    out.frames++;
  }
  out.last = last;

  // A second view of the same stream counts its own frames. At one frame a
  // second its third is a second away when the view closes, so the request
  // for it is still waiting then, also on a slow runner.
  const other = (await ipc.invoke("media_view_open", { testPattern: pattern, maxFps: 1 })).view;
  out.otherFirst = await frame(other);
  out.paced = await frame(other, out.otherFirst.seq);
  const waiting = error(ipc.invoke("media_view_frame", { view: other, after: out.paced.seq }));
  await ipc.invoke("media_view_close", { view: other });
  out.closedWhileWaiting = await waiting;
  out.afterClose = await error(ipc.invoke("media_view_frame", { view: other }));

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
    check_frame("second frame of the second view", video.get("paced"), 640, 360, problems)
    if not video.get("inOrder") or video.get("frames", 0) < 5:
        problems.append(f"{video.get('frames')} frames in a second, in order: {video.get('inOrder')}")
    names = ("first", "second", "grown", "last", "otherFirst", "paced")
    frames = {name: video.get(name) if isinstance(video.get(name), dict) else {} for name in names}
    seq = {name: frame.get("seq", 0) for name, frame in frames.items()}
    # Numbers start at 1 and grow (a slow page misses frames, so not by one);
    # the second view starts over, below where the first one was by then.
    if not (1 <= seq["first"] < seq["second"] < seq["grown"] < seq["last"]
            and 1 <= seq["otherFirst"] < seq["paced"] < seq["last"]):
        problems.append(f"sequence numbers of the views are {seq}")
    # At one frame a second, the second frame is due half a second after the
    # first (frames.rs, Pace).
    apart = (frames["paced"].get("timestampUs", 0) - frames["otherFirst"].get("timestampUs", 0)) / 1e6
    if apart < 0.4:
        problems.append(f"frames of the view limited to 1 fps are {apart:.3f} s apart")
    for name in ("closedWhileWaiting", "afterClose", "closed", "neither", "both", "unknownConsumer",
                 "unknownSource", "unknownView"):
        if not video.get(name):
            problems.append(f"video: {name} did not fail")


def check_windows_stubs(report, problems):
    apps = report.get("audioApps", {})
    if apps != {"ok": []}:
        problems.append(f"media_audio_apps on Windows: {apps}, expected an empty list")
    for name, what in (("screen", "screen capture"), ("appAudio", "application sound")):
        answer = report.get(name, {})
        expected = f"{what} is not available on Windows yet"
        if expected not in str(answer.get("error", "")):
            problems.append(f"{name} on Windows: {answer}, expected the error {expected!r}")


class Handler(BaseHTTPRequestHandler):
    def do_GET(self):
        self.server.gets += 1
        if self.server.created is not None and self.server.gets == 2:
            time.sleep(HOLD_SECOND)
        # The first page leaves a view open when its web process is to end;
        # the page after it is told which.
        left = self.server.left
        leave = self.server.created is not None and left is None
        page = PAGE.replace(b"__LEAVE__", b"true" if leave else b"false")
        page = page.replace(b"__LEFT__", b"null" if left is None else str(left).encode())
        page = page.replace(b"__WINDOWS__", b"true" if WINDOWS else b"false")
        self.send_response(200)
        self.send_header("Content-Type", "text/html; charset=utf-8")
        # Loaded again, the page has to come from here: it differs.
        self.send_header("Cache-Control", "no-store")
        self.end_headers()
        self.wfile.write(page)

    def do_POST(self):
        length = int(self.headers.get("Content-Length", "0"))
        report = json.loads(self.rfile.read(length) or b"{}")
        self.send_response(204)
        self.end_headers()
        print("report:", json.dumps(report, indent=2), flush=True)
        problems = []
        back = self.server.left is not None
        if back and "unknown view" not in str(report.get("leftover")):
            problems.append(f"the page that came back still reaches the view left open: {report.get('leftover')}")
        if not report.get("ipc"):
            problems.append("no Tauri IPC on the server page")
        info = report.get("info", {}).get("ok", {})
        if info.get("abi") != int(sys.argv[2]):
            problems.append("media_info did not answer with the core's ABI")
        features = info.get("features")
        if not isinstance(features, list) or not all(isinstance(f, str) for f in features):
            problems.append("media_info did not list the app's features")
        if "inputs" not in report.get("devices", {}).get("ok", {}):
            problems.append("media_audio_devices failed")
        if "ok" not in report.get("levels", {}):
            problems.append("media_audio_levels failed")
        if "ok" in report.get("setServer", {}):
            problems.append("the server origin may call set_server")
        if WINDOWS:
            check_windows_stubs(report, problems)
        check_video(report.get("video", {}), problems)
        leaves = self.server.created is not None and not back
        if leaves and not isinstance(report.get("left"), int):
            problems.append(f"the view to leave open: {report.get('left')}")
        for problem in problems:
            print("FAIL:", problem, flush=True)
        if leaves and not problems:
            # Not the end: the caller ends the web process now.
            self.server.left = report["left"]
            self.server.back_by = time.monotonic() + BACK_WITHIN
            self.server.created.touch()
            print("first report good; waiting for the page to come back", flush=True)
            return
        self.server.result = 1 if problems else 0

    def log_message(self, *args):
        pass


# A thread per connection: Chromium (WebView2) opens connections ahead of use,
# and one that stays idle must not hold up the report.
server = ThreadingHTTPServer(("127.0.0.1", int(sys.argv[1])), Handler)
server.result = None
server.timeout = 1
# The file that tells the caller to end the web process, if it asked for that;
# the view the first page left open; when the page has to be back.
server.created = pathlib.Path(sys.argv[3]) if len(sys.argv) > 3 else None
server.left = None
server.back_by = None
server.gets = 0
while server.result is None:
    server.handle_request()
    if server.result is None and server.back_by is not None and time.monotonic() > server.back_by:
        print(f"FAIL: the page did not come back within {BACK_WITHIN} s of its web process ending", flush=True)
        server.result = 1
sys.exit(server.result)
