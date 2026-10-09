"""Stand-in Gelabber origin for the desktop app's smoke test.

Serves a page that calls the media commands through Tauri's IPC the way the
web client will, plus one command reserved for the bundled setup page, and
checks the report the page posts back. Exits 0 when the server origin reaches
the media core and nothing else.
"""

import json
import sys
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
  }
  await fetch("/report", { method: "POST", body: JSON.stringify(report) });
})();
</script></body></html>"""


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
