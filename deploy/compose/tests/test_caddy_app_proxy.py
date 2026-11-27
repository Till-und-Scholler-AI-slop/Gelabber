"""Regression tests for the app-facing Caddy sites (bundled and homelab).

The API accepts a browser WebSocket only when the host[:port] of `Origin`
equals the `Host` header it receives, the scheme's default port aside, and
behind TLS (API_COOKIE_SECURE) only from an `https://` page
(backend/api/src/gateway/mod.rs, origin_allowed).
Caddy's `{host}` placeholder has no port, so forwarding it turns every `/ws`
handshake into a 403 as soon as the stack is opened on a non-default port
(GELABBER_HTTP_PORT, or a TLS proxy on :8443). Every app upstream must
therefore receive `{hostport}`.

Both sites also compress responses: the API does not compress its JSON, and a
phone on mobile data pays for every byte.

The API keys its rate limits by the rightmost `X-Forwarded-For` entry (#287),
and media its cap on `/ws` sockets that have not joined a call (#286). Every
API and media upstream therefore gets exactly one entry, the client Caddy
resolved (`{client_ip}`), and the bundled Caddy believes the proxies in
GELABBER_TRUSTED_PROXIES (Compose default `private_ranges`, strict mode), so
users behind a proxy in front do not all share that proxy's window.
"""
from __future__ import annotations

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
API_UPSTREAMS = {"Caddyfile": "api:8080", "Caddyfile.homelab": "127.0.0.1:18080"}
MEDIA_UPSTREAMS = {"Caddyfile": "media:8081", "Caddyfile.homelab": "127.0.0.1:18081"}
CLIENT_IP = "{http.vars.client_ip}"


def _servers(adapted: dict[str, Any]) -> dict[str, Any]:
    return adapted.get("apps", {}).get("http", {}).get("servers", {})


def _forwarded_for_values(proxy: dict[str, Any]) -> list[str]:
    return list(
        proxy.get("headers", {})
        .get("request", {})
        .get("set", {})
        .get("X-Forwarded-For", [])
    )


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


class OperatorGuideTest(unittest.TestCase):
    """What an operator copies into a Caddy of their own; runs without caddy."""

    def test_no_guide_recommends_host_without_port(self) -> None:
        for path in OPERATOR_GUIDES:
            with self.subTest(path.name):
                self.assertFalse(
                    "header_up Host {host}" in path.read_text(encoding="utf-8"),
                    "an operator's own Caddy must forward {hostport} as well",
                )


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

    def test_api_and_media_upstreams_receive_the_resolved_client_only(self) -> None:
        for name, adapted in self.adapted.items():
            with self.subTest(name):
                api = 0
                media = 0
                for proxy in _iter_handlers(adapted):
                    dials = {u.get("dial") for u in proxy.get("upstreams", [])}
                    if API_UPSTREAMS[name] in dials:
                        api += 1
                        self.assertEqual(_forwarded_for_values(proxy), [CLIENT_IP])
                    elif MEDIA_UPSTREAMS[name] in dials:
                        media += 1
                        self.assertEqual(_forwarded_for_values(proxy), [CLIENT_IP])
                    else:
                        self.assertEqual(_forwarded_for_values(proxy), [], sorted(dials))
                self.assertEqual(api, 4)
                self.assertEqual(media, 1)

    def test_bundled_caddy_trusts_only_the_configured_proxies(self) -> None:
        source = (COMPOSE / "Caddyfile").read_text(encoding="utf-8")
        cases = {
            # Explicitly empty in .env: nobody may report a client address.
            "": set(),
            "private_ranges 203.0.113.0/24": {"10.0.0.0/8", "203.0.113.0/24"},
        }
        for value, expected in cases.items():
            with self.subTest(GELABBER_TRUSTED_PROXIES=value):
                adapted = _adapt(
                    source, self.caddy_bin, {"GELABBER_TRUSTED_PROXIES": value}
                )
                servers = _servers(adapted)
                self.assertTrue(servers)
                for server in servers.values():
                    trusted = server.get("trusted_proxies", {})
                    self.assertEqual(trusted.get("source"), "static")
                    ranges = set(trusted.get("ranges") or [])
                    if expected:
                        self.assertTrue(expected <= ranges, sorted(ranges))
                    else:
                        self.assertEqual(ranges, set())
                    self.assertTrue(server.get("trusted_proxies_strict"))


if __name__ == "__main__":
    unittest.main()
