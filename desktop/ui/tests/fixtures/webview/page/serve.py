"""Serves the fixture page as a Gelabber server would its web client.

    python3 -I serve.py <port> <done-file> [fixed]

Binds 127.0.0.1:<port>, serves this directory and creates <done-file> once
the page has written its second phase (GET /done). With "fixed", the root
redirects to /?at=fixed: an app that loads the server's root writes drafts
dated for the unit tests' clock. Standard library only.
"""

import http.server
import pathlib
import sys

HERE = pathlib.Path(__file__).resolve().parent


def main() -> None:
    port, done = int(sys.argv[1]), pathlib.Path(sys.argv[2])
    fixed = sys.argv[3:] == ["fixed"]

    class Handler(http.server.SimpleHTTPRequestHandler):
        def __init__(self, *args, **kwargs):
            super().__init__(*args, directory=str(HERE), **kwargs)

        def do_GET(self):
            if self.path.split("?")[0] == "/done":
                done.write_text("done\n", encoding="utf-8")
                self.send_response(204)
                self.end_headers()
                return
            if fixed and self.path == "/":
                self.send_response(302)
                self.send_header("Location", "/?at=fixed")
                self.end_headers()
                return
            super().do_GET()

    http.server.ThreadingHTTPServer(("127.0.0.1", port), Handler).serve_forever()


if __name__ == "__main__":
    main()
