"""Regression tests for the browser-facing MinIO presign endpoint.

The API signs MINIO_PUBLIC_ENDPOINT into every PUT URL.  The host Caddy
configuration therefore needs a separate vhost for that exact hostname; the
application vhost must never receive the upload request.
"""
from pathlib import Path
import re
import unittest


ROOT = Path(__file__).resolve().parents[2]
CADDY = ROOT / "compose" / "Caddyfile.homelab"


class MinioProxyConfigTest(unittest.TestCase):
    def test_public_minio_vhost_is_active_and_preserves_host(self) -> None:
        config = CADDY.read_text()
        # A commented example (the pre-#69 configuration) must not satisfy
        # this check.  The vhost is intentionally the final site block.
        match = re.search(
            r"(?m)^\{\$GELABBER_MINIO_DOMAIN:minio\.home\.arpa\}\s*\{(?P<body>[\s\S]*)\}\s*$",
            config,
        )
        self.assertIsNotNone(match, "MINIO_PUBLIC_ENDPOINT has no active Caddy vhost")
        body = match.group("body")
        self.assertIn(
            "reverse_proxy {$GELABBER_MINIO_UPSTREAM:127.0.0.1:9000}", body
        )
        self.assertIn("header_up Host {host}", body)


if __name__ == "__main__":
    unittest.main()
