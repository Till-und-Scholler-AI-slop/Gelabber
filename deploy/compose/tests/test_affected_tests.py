"""Inside an area, CI tests what a change can affect, and everything when in
doubt (.github/workflows/scripts/rust_tests.py, GEL-393).

The crates and their dependencies come from the repository's own manifests,
so these tests follow the workspace when a crate gains a dependency.
"""
import importlib.util
import io
from pathlib import Path
import re
import shutil
import sys
import tempfile
import tomllib
import unittest
from unittest import mock

ROOT = Path(__file__).resolve().parents[3]
SCRIPT = ROOT / '.github/workflows/scripts/rust_tests.py'


def load():
    spec = importlib.util.spec_from_file_location('rust_tests', SCRIPT)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


rust_tests = load()
WORKSPACE = rust_tests.crates()


def selected(*paths, event='pull_request'):
    """The `$GITHUB_OUTPUT` lines ci.yml gets for these changed paths."""
    out = io.StringIO()
    with mock.patch.object(sys, 'stdin', io.StringIO('\n'.join(paths) + '\n')), \
            mock.patch.object(sys, 'stdout', out), mock.patch.object(sys, 'stderr', io.StringIO()):
        rust_tests.main(['--event', event])
    return dict(line.split('=', 1) for line in out.getvalue().splitlines())


def runs(*paths, event='pull_request'):
    """The crates whose tests run, or 'all'."""
    lines = selected(*paths, event=event)
    if lines['all'] == 'true':
        assert lines['filter'] == 'all()'
        return 'all'
    return set(re.findall(r'package\(([^)]+)\)', lines['filter']))


class Backend(unittest.TestCase):
    def test_the_workspace_is_read_from_its_manifests(self):
        self.assertEqual(set(WORKSPACE), {'gelabber-api', 'gelabber-media', 'gelabber-shared'})

    def test_a_change_in_shared_runs_api_and_media(self):
        self.assertEqual(runs('backend/shared/src/lib.rs'),
                         {'gelabber-shared', 'gelabber-api', 'gelabber-media'})

    def test_a_change_in_api_does_not_run_media(self):
        self.assertEqual(runs('backend/api/src/gateway/conn.rs', 'backend/api/tests/gateway.rs'),
                         {'gelabber-api'})
        self.assertEqual(runs('backend/media/src/lib.rs'), {'gelabber-media'})

    def test_other_areas_add_nothing(self):
        self.assertEqual(runs('backend/api/src/lib.rs', 'web/src/main.tsx', 'desktop/ui/src/app.rs', 'docs/ci.md'),
                         {'gelabber-api'})

    def test_whatever_every_crate_is_built_with_runs_all(self):
        for path in ('backend/Cargo.lock', 'backend/Cargo.toml', 'backend/api/Cargo.toml', 'rust-toolchain.toml',
                     '.github/workflows/ci.yml', '.github/workflows/scripts/rust_tests.py',
                     'backend/.config/nextest.toml', 'backend/build/run-native-ci.sh', '.dockerignore'):
            with self.subTest(path=path):
                self.assertEqual(runs('backend/api/src/lib.rs', path), 'all')

    def test_runs_all_when_in_doubt(self):
        self.assertEqual(runs(), 'all')
        self.assertEqual(runs('web/src/main.tsx'), 'all')
        for event in ('push', 'merge_group', 'workflow_dispatch'):
            with self.subTest(event=event):
                self.assertEqual(runs('backend/api/src/lib.rs', event=event), 'all')

    def test_unreadable_manifests_run_all(self):
        with tempfile.TemporaryDirectory() as tmp:
            shutil.copytree(ROOT / 'backend', Path(tmp, 'backend'),
                            ignore=lambda folder, names: [name for name in names if name == 'target'])
            Path(tmp, 'backend/media/Cargo.toml').write_text('[package\n')
            with mock.patch.object(rust_tests, 'ROOT', Path(tmp)):
                self.assertEqual(runs('backend/api/src/lib.rs'), 'all')

    def test_a_new_dependent_crate_is_run_with_its_dependency(self):
        with tempfile.TemporaryDirectory() as tmp:
            shutil.copytree(ROOT / 'backend', Path(tmp, 'backend'),
                            ignore=lambda folder, names: [name for name in names if name == 'target'])
            manifest = Path(tmp, 'backend/Cargo.toml')
            manifest.write_text(manifest.read_text().replace('"shared"]', '"shared", "tools"]', 1))
            Path(tmp, 'backend/tools').mkdir()
            Path(tmp, 'backend/tools/Cargo.toml').write_text(
                '[package]\nname = "gelabber-tools"\n[dev-dependencies]\napi = { package = "gelabber-api", path = "../api" }\n')
            with mock.patch.object(rust_tests, 'ROOT', Path(tmp)):
                self.assertEqual(runs('backend/api/src/lib.rs'), {'gelabber-api', 'gelabber-tools'})


class Workflows(unittest.TestCase):
    def test_the_rust_job_runs_what_the_selection_names(self):
        ci = (ROOT / '.github/workflows/ci.yml').read_text()
        job = ci.split('\n  rust:\n', 1)[1].split('\n  deploy:\n', 1)[0]
        self.assertIn('fetch-depth: 2', job)
        self.assertIn('rust_tests.py --event "$EVENT"', job)
        self.assertIn('SELECTED: ${{ steps.select.outputs.filter }}', job)
        self.assertIn('-E "($SELECTED) and not test(typing_broadcasts_fast_and_clears_on_stop)"', job)
        typing = job.split('- name: Test typing latency in isolation\n', 1)[1].split('\n', 1)[0]
        self.assertIn("steps.select.outputs.all == 'true'", typing)
        self.assertIn("contains(fromJSON(steps.select.outputs.packages), 'gelabber-api')", typing)

    def test_desktop_media_tests_run_alone_and_the_others_side_by_side(self):
        native = (ROOT / '.github/workflows/desktop-native.yml').read_text()
        profiles = tomllib.loads((ROOT / 'desktop/.config/nextest.toml').read_text())['profile']
        self.assertEqual(profiles['ci-media']['test-threads'], 1)
        self.assertNotIn('test-threads', profiles['ci'])
        media = re.search(r'MEDIA_TESTS: (.*)\n', native).group(1)
        self.assertIn('package(gelabber-media-core)', media)
        self.assertIn('--profile ci-media --locked --manifest-path desktop/Cargo.toml --no-fail-fast -E "$MEDIA_TESTS"',
                      native)
        self.assertIn('--profile ci --locked --manifest-path desktop/Cargo.toml --no-fail-fast -E "not ($MEDIA_TESTS)"',
                      native)


if __name__ == '__main__':
    unittest.main()
