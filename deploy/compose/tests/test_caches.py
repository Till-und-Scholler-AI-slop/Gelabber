"""The Actions caches cache-cleanup.yml deletes (.github/workflows/scripts/caches.py)."""
import importlib.util
from pathlib import Path
import unittest

ROOT = Path(__file__).resolve().parents[3]
SCRIPT = ROOT / '.github/workflows/scripts/caches.py'


def load():
    spec = importlib.util.spec_from_file_location('caches', SCRIPT)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


caches = load()
DEBIAN = 'v0-rust-native-debian-af417170b5b2208ed4e554cb708ffb466c341721e756cf990843b331916bc5d2-Linux-x64'


def cache(number, key, ref='refs/heads/main', created='2026-11-24T00:00:00Z'):
    return {'id': number, 'key': key, 'ref': ref, 'created_at': created, 'size_in_bytes': 10**9}


def deleted(entries, development='v0.14'):
    return sorted(cache_id for cache_id, *_ in caches.superseded(entries, development))


class KindTest(unittest.TestCase):
    def test_generations_of_one_cache_share_a_kind(self):
        self.assertEqual(caches.kind('v0-rust-1-core-windows-Windows_NT-x64-744a7afe-578b7d58'),
                         caches.kind('v0-rust-1-core-windows-Windows_NT-x64-744a7afe-8bbe2e12'))
        self.assertEqual(caches.kind(DEBIAN), 'v0-rust-native-debian-Linux-x64')
        # The key once the environment and Cargo.lock hashes are in it (GEL-355).
        self.assertEqual(caches.kind(DEBIAN + '-0123abcd-89abcdef'), caches.kind(DEBIAN))

    def test_different_caches_have_different_kinds(self):
        self.assertNotEqual(caches.kind('v0-rust-core-linux-Linux-x64-2d3e5bdd-947bdb6b'),
                            caches.kind('v0-rust-1-core-windows-Windows_NT-x64-744a7afe-578b7d58'))

    def test_buildkit_layers_are_left_alone(self):
        self.assertIsNone(caches.kind('index-media-1-f921bd05#28'))
        self.assertIsNone(caches.kind('buildkit-blob-1-sha256:0123456789abcdef'))


class SupersededTest(unittest.TestCase):
    def test_only_the_newest_generation_stays(self):
        entries = [cache(1, 'v0-rust-core-linux-Linux-x64-2d3e5bdd-12f24d94', created='2026-11-23T14:54:37Z'),
                   cache(2, 'v0-rust-core-linux-Linux-x64-2d3e5bdd-947bdb6b', created='2026-11-24T12:51:08Z')]
        self.assertEqual(deleted(entries), [1])

    def test_main_and_the_development_line_keep_their_own(self):
        key = 'v0-rust-core-linux-Linux-x64-2d3e5bdd-947bdb6b'
        self.assertEqual(deleted([cache(1, key), cache(2, key, ref='refs/heads/v0.14')]), [])

    def test_older_version_branches_and_pull_requests_go(self):
        key = 'v0-rust-1-core-windows-Windows_NT-x64-744a7afe-578b7d58'
        entries = [cache(1, key, ref='refs/heads/v0.13'),
                   cache(2, 'node-cache-Linux-x64-npm-5f98175b', ref='refs/pull/748/merge'),
                   cache(3, key, ref='refs/heads/v0.14')]
        self.assertEqual(deleted(entries), [1, 2])

    def test_other_branches_and_families_stay(self):
        entries = [cache(1, 'index-media-1-f921bd05#28'), cache(2, 'index-media-1-f921bd05#29'),
                   cache(3, 'v0-rust-core-linux-Linux-x64-2d3e5bdd-947bdb6b', ref='refs/heads/release/v0.14.0')]
        self.assertEqual(deleted(entries), [])

    def test_without_a_version_branch_main_still_keeps_its_newest(self):
        entries = [cache(1, 'libwebrtc-linux-x64-dfaec169', created='2026-11-12T00:00:00Z'),
                   cache(2, 'libwebrtc-linux-x64-0badc0de', created='2026-11-20T00:00:00Z')]
        self.assertEqual(deleted(entries, development=None), [1])


class DevelopmentLineTest(unittest.TestCase):
    def test_the_newest_version_branch_by_number(self):
        self.assertEqual(caches.development_line(['main', 'v0.9', 'v0.14', 'v0.13', 'release/v0.15.0']), 'v0.14')
        self.assertIsNone(caches.development_line(['main', 'release/v0.15.0']))


if __name__ == '__main__':
    unittest.main()
