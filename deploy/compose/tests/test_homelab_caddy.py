"""Regression tests for the browser-facing MinIO presign Caddy vhost.

The API signs MINIO_PUBLIC_ENDPOINT into every PUT/GET URL. Homelab Caddy
must expose that exact hostname on an active site, proxy to MinIO without
rewriting the URI, and forward Host including any non-default port.
"""
from __future__ import annotations

import json
import os
import re
import shutil
import subprocess
import tempfile
import unittest
from pathlib import Path
from typing import Any, Iterator


ROOT = Path(__file__).resolve().parents[2]
CADDYFILE = ROOT / "compose" / "Caddyfile.homelab"
MINIO_HOST = "minio.home.arpa"
MINIO_DIAL = "127.0.0.1:9000"
HOSTPORT = "{http.request.hostport}"
HOST_ONLY = "{http.request.host}"
UPSTREAM_HOSTPORT = "{http.reverse_proxy.upstream.hostport}"


def _find_caddy() -> str:
    env = os.environ.get("CADDY_BIN")
    if env and Path(env).is_file() and os.access(env, os.X_OK):
        return env
    which = shutil.which("caddy")
    if which:
        return which
    raise unittest.SkipTest(
        "caddy binary required for adapt tests (install caddy or set CADDY_BIN)"
    )


def _adapt(caddyfile_text: str, caddy_bin: str) -> dict[str, Any]:
    with tempfile.NamedTemporaryFile(
        "w", suffix=".Caddyfile", delete=False, encoding="utf-8"
    ) as fh:
        fh.write(caddyfile_text)
        path = fh.name
    try:
        proc = subprocess.run(
            [caddy_bin, "adapt", "--config", path, "--adapter", "caddyfile"],
            check=False,
            capture_output=True,
            text=True,
        )
        if proc.returncode != 0:
            raise AssertionError(
                f"caddy adapt failed ({proc.returncode}): {proc.stderr.strip()}"
            )
        return json.loads(proc.stdout)
    finally:
        Path(path).unlink(missing_ok=True)


def _iter_handlers(node: Any) -> Iterator[dict[str, Any]]:
    if isinstance(node, dict):
        if node.get("handler") == "reverse_proxy":
            yield node
        for value in node.values():
            yield from _iter_handlers(value)
    elif isinstance(node, list):
        for item in node:
            yield from _iter_handlers(item)


def _find_host_route(adapted: dict[str, Any], hostname: str) -> dict[str, Any] | None:
    servers = adapted.get("apps", {}).get("http", {}).get("servers", {})
    for server in servers.values():
        for route in server.get("routes", []):
            for matcher in route.get("match", []):
                if hostname in matcher.get("host", []):
                    return route
    return None


def _minio_proxy(route: dict[str, Any]) -> dict[str, Any] | None:
    proxies = list(_iter_handlers(route))
    return proxies[0] if proxies else None


def _host_header_values(proxy: dict[str, Any]) -> list[str]:
    return list(
        proxy.get("headers", {})
        .get("request", {})
        .get("set", {})
        .get("Host", [])
    )


def assert_effective_minio_presign_config(
    test: unittest.TestCase, adapted: dict[str, Any]
) -> None:
    """Assert MinIO site matcher, upstream, Host+port, and unchanged URI."""
    route = _find_host_route(adapted, MINIO_HOST)
    test.assertIsNotNone(
        route, f"no active Caddy route matching host {MINIO_HOST!r}"
    )
    assert route is not None
    proxy = _minio_proxy(route)
    test.assertIsNotNone(proxy, "MinIO site has no reverse_proxy handler")
    assert proxy is not None

    dials = [u.get("dial") for u in proxy.get("upstreams", [])]
    test.assertIn(
        MINIO_DIAL,
        dials,
        f"MinIO upstream dial missing {MINIO_DIAL!r}; got {dials!r}",
    )
    # Default reverse_proxy keeps method+URI; an explicit rewrite would break
    # path-signed S3 URLs.
    test.assertNotIn("rewrite", proxy)
    test.assertNotIn("rewrite_method", proxy)

    host_vals = _host_header_values(proxy)
    test.assertEqual(
        host_vals,
        [HOSTPORT],
        "Host must be forwarded as {hostport} (incl. non-default port); "
        f"got {host_vals!r}",
    )


def _replace_minio_site(src: str, body: str) -> str:
    pattern = re.compile(
        r"(?ms)^\{\$GELABBER_MINIO_DOMAIN:minio\.home\.arpa\}\s*\{.*?^\}\s*",
    )
    replacement = (
        "{$GELABBER_MINIO_DOMAIN:minio.home.arpa} {\n"
        + body
        + "\n}\n"
    )
    out, n = pattern.subn(replacement, src, count=1)
    if n != 1:
        raise AssertionError("could not replace MinIO site block in source")
    return out


class MinioProxyConfigTest(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls.caddy_bin = _find_caddy()
        cls.source = CADDYFILE.read_text(encoding="utf-8")

    def test_effective_minio_vhost_preserves_hostport_and_uri(self) -> None:
        adapted = _adapt(self.source, self.caddy_bin)
        assert_effective_minio_presign_config(self, adapted)

    def test_site_block_order_does_not_matter(self) -> None:
        # Move the MinIO site block before the app site; effective JSON must
        # still satisfy the same assertions (no fixed source-line requirement).
        minio_match = re.search(
            r"(?ms)^\{\$GELABBER_MINIO_DOMAIN:minio\.home\.arpa\}\s*\{.*?^\}\s*",
            self.source,
        )
        self.assertIsNotNone(minio_match)
        assert minio_match is not None
        without = self.source[: minio_match.start()] + self.source[minio_match.end() :]
        reordered = minio_match.group(0) + "\n" + without
        adapted = _adapt(reordered, self.caddy_bin)
        assert_effective_minio_presign_config(self, adapted)

    def test_commented_reverse_proxy_does_not_pass(self) -> None:
        broken = _replace_minio_site(
            self.source,
            "\t# reverse_proxy {$GELABBER_MINIO_UPSTREAM:127.0.0.1:9000} {\n"
            "\t#\theader_up Host {hostport}\n"
            "\t# }\n",
        )
        # Empty site is invalid or has no proxy — either way must fail the assert.
        try:
            adapted = _adapt(broken, self.caddy_bin)
        except AssertionError:
            return
        with self.assertRaises(AssertionError):
            assert_effective_minio_presign_config(self, adapted)

    def test_missing_reverse_proxy_fails(self) -> None:
        broken = _replace_minio_site(self.source, "\trespond \"no proxy\" 200\n")
        adapted = _adapt(broken, self.caddy_bin)
        with self.assertRaises(AssertionError):
            assert_effective_minio_presign_config(self, adapted)

    def test_host_only_override_fails_hostport_check(self) -> None:
        broken = _replace_minio_site(
            self.source,
            "\treverse_proxy {$GELABBER_MINIO_UPSTREAM:127.0.0.1:9000} {\n"
            "\t\theader_up Host {host}\n"
            "\t}\n",
        )
        adapted = _adapt(broken, self.caddy_bin)
        with self.assertRaises(AssertionError):
            assert_effective_minio_presign_config(self, adapted)
        # Sanity: adapt did produce {host}, not {hostport}.
        route = _find_host_route(adapted, MINIO_HOST)
        assert route is not None
        proxy = _minio_proxy(route)
        assert proxy is not None
        self.assertEqual(_host_header_values(proxy), [HOST_ONLY])

    def test_upstream_host_override_fails_hostport_check(self) -> None:
        broken = _replace_minio_site(
            self.source,
            "\treverse_proxy {$GELABBER_MINIO_UPSTREAM:127.0.0.1:9000} {\n"
            "\t\theader_up Host {upstream_hostport}\n"
            "\t}\n",
        )
        adapted = _adapt(broken, self.caddy_bin)
        with self.assertRaises(AssertionError):
            assert_effective_minio_presign_config(self, adapted)
        route = _find_host_route(adapted, MINIO_HOST)
        assert route is not None
        proxy = _minio_proxy(route)
        assert proxy is not None
        self.assertEqual(_host_header_values(proxy), [UPSTREAM_HOSTPORT])

    def test_commented_host_forward_lines_do_not_satisfy_hostport(self) -> None:
        # The old assertIn/regex suite stayed green when only the Host lines
        # were commented. Effective config must fail the hostport assertion
        # (no Host set → empty list ≠ {hostport}).
        broken = _replace_minio_site(
            self.source,
            "\treverse_proxy {$GELABBER_MINIO_UPSTREAM:127.0.0.1:9000} {\n"
            "\t\t# header_up Host {hostport}\n"
            "\t\t# header_up Host {host}\n"
            "\t}\n",
        )
        adapted = _adapt(broken, self.caddy_bin)
        with self.assertRaises(AssertionError):
            assert_effective_minio_presign_config(self, adapted)


if __name__ == "__main__":
    unittest.main()
