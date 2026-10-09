"""Regression tests for the app-facing Caddy sites (bundled and homelab).

The API accepts a browser WebSocket only when the host[:port] of `Origin`
equals the `Host` header it receives, the scheme's default port aside, and
behind TLS (API_COOKIE_SECURE) only from an `https://` page
(api/src/gateway/mod.rs, origin_allowed).
Caddy's `{host}` placeholder has no port, so forwarding it turns every `/ws`
handshake into a 403 as soon as the stack is opened on a non-default port
(GELABBER_HTTP_PORT, or a TLS proxy on :8443). Every app upstream must
therefore receive `{hostport}`.

Both sites also compress responses: the API does not compress its JSON, and a
phone on mobile data pays for every byte.
"""
from __future__ import annotations

import re
import unittest
from pathlib import Path
from typing import Any

from test_homelab_caddy import (
    HOSTPORT,
    _adapt,
    _find_caddy,
    _host_header_values,
    _iter_handlers,
)


COMPOSE = Path(__file__).resolve().parents[1]
CADDYFILES = (COMPOSE / "Caddyfile", COMPOSE / "Caddyfile.homelab")
# What an operator copies into a Caddy of their own.
OPERATOR_GUIDES = (
    COMPOSE.parent / "README.md",
    COMPOSE / ".env.example",
    COMPOSE / ".env.homelab.example",
    *CADDYFILES,
)
APP_UPSTREAMS = {
    "Caddyfile": {"api:8080", "media:8081", "web:80"},
    "Caddyfile.homelab": {"127.0.0.1:18080", "127.0.0.1:18081", "127.0.0.1:8088"},
}


def _active_lines(text: str) -> list[str]:
    return [
        line.strip()
        for line in text.splitlines()
        if line.strip() and not line.strip().startswith("#")
    ]


def _encode_handlers(node: Any) -> list[dict[str, Any]]:
    found: list[dict[str, Any]] = []
    if isinstance(node, dict):
        if node.get("handler") == "encode":
            found.append(node)
        for value in node.values():
            found.extend(_encode_handlers(value))
    elif isinstance(node, list):
        for item in node:
            found.extend(_encode_handlers(item))
    return found


class CaddyfileTextTest(unittest.TestCase):
    """Runs everywhere, without a caddy binary."""

    def test_every_proxy_forwards_host_with_port(self) -> None:
        for path in CADDYFILES:
            with self.subTest(path.name):
                lines = _active_lines(path.read_text(encoding="utf-8"))
                proxies = [line for line in lines if line.startswith("reverse_proxy ")]
                forwards = [line for line in lines if line.startswith("header_up Host ")]
                self.assertGreaterEqual(len(proxies), 6)
                self.assertEqual(
                    forwards,
                    ["header_up Host {hostport}"] * len(proxies),
                    "each reverse_proxy must forward Host as {hostport}; "
                    "{host} drops a non-default port and /ws answers 403",
                )

    def test_no_guide_recommends_host_without_port(self) -> None:
        for path in OPERATOR_GUIDES:
            with self.subTest(path.name):
                self.assertFalse(
                    "header_up Host {host}" in path.read_text(encoding="utf-8"),
                    "an operator's own Caddy must forward {hostport} as well",
                )

    def test_responses_are_compressed(self) -> None:
        for path in CADDYFILES:
            with self.subTest(path.name):
                lines = _active_lines(path.read_text(encoding="utf-8"))
                encode = [line for line in lines if re.match(r"encode\b", line)]
                self.assertEqual(len(encode), 1, encode)
                self.assertIn("gzip", encode[0].split())


class CaddyfileEffectiveConfigTest(unittest.TestCase):
    """What Caddy actually runs, so a commented or misplaced line cannot pass."""

    @classmethod
    def setUpClass(cls) -> None:
        cls.caddy_bin = _find_caddy()
        cls.adapted = {
            path.name: _adapt(path.read_text(encoding="utf-8"), cls.caddy_bin)
            for path in CADDYFILES
        }

    def test_app_upstreams_receive_host_with_port(self) -> None:
        for name, adapted in self.adapted.items():
            with self.subTest(name):
                seen: set[str] = set()
                for proxy in _iter_handlers(adapted):
                    dials = {u.get("dial") for u in proxy.get("upstreams", [])}
                    seen |= dials
                    self.assertEqual(
                        _host_header_values(proxy),
                        [HOSTPORT],
                        f"upstream {sorted(dials)} must receive Host as {{hostport}}",
                    )
                self.assertTrue(
                    APP_UPSTREAMS[name] <= seen,
                    f"missing app upstreams: {sorted(APP_UPSTREAMS[name] - seen)}",
                )

    def test_app_site_compresses_with_gzip(self) -> None:
        for name, adapted in self.adapted.items():
            with self.subTest(name):
                encoders = _encode_handlers(adapted)
                self.assertEqual(len(encoders), 1, encoders)
                self.assertIn("gzip", encoders[0].get("encodings", {}))


if __name__ == "__main__":
    unittest.main()
