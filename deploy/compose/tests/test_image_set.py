"""Mocked workflow/registry regressions; no GHCR publication or Docker runtime."""
import importlib.util
import json
import os
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[3]
spec = importlib.util.spec_from_file_location('image_set', ROOT / '.github/workflows/scripts/image_set.py')
m = importlib.util.module_from_spec(spec)
spec.loader.exec_module(m)
SHA = 'a' * 40
DIGEST = 'sha256:' + 'b' * 64
OTHER = 'sha256:' + 'c' * 64
REPO = 'owner/gelabber'


def ci(**changes):
    record = dict(head_sha=SHA, head_branch='main', event='push', status='completed',
                  conclusion='success', path='.github/workflows/ci.yml',
                  repository={'full_name': REPO}, head_repository={'full_name': REPO})
    return {**record, **changes}


class Gates(unittest.TestCase):
    def test_ci_caddy_uses_compose_pin_and_checks_downloaded_version(self):
        compose = (ROOT / 'deploy/compose/compose.yaml').read_text()
        workflow = (ROOT / '.github/workflows/ci.yml').read_text()
        import re
        version = re.search(r'image: caddy:([^:]+)-alpine', compose).group(1)
        self.assertIn('&version=v' + version, workflow)
        self.assertIn('test "$(caddy version | cut -d \' \' -f 1)" = "v' + version + '"', workflow)

    def test_exact_success(self):
        m.check_ci(ci(), SHA, REPO)

    def test_failed_pending_foreign_or_wrong_sha_ci(self):
        for changes in ({'conclusion': 'failure'}, {'conclusion': 'cancelled'},
                        {'status': 'in_progress'}, {'head_sha': 'd' * 40},
                        {'event': 'pull_request'}, {'head_branch': 'feature'},
                        {'path': '.github/workflows/other.yml'},
                        {'head_repository': {'full_name': 'foreign/fork'}}):
            with self.subTest(changes=changes), self.assertRaises(ValueError):
                m.check_ci(ci(**changes), SHA, REPO)

    def test_unchanged_version_or_obsolete_main_not_promoted(self):
        self.assertFalse(m.promotion_allowed(SHA, SHA, 'd' * 40))
        self.assertFalse(m.promotion_allowed(SHA, 'd' * 40, None))
        self.assertTrue(m.promotion_allowed(SHA, SHA, SHA))
        self.assertTrue(m.promotion_allowed(SHA, SHA, None))

    def test_registry_errors_fail_closed(self):
        for message in ('unauthorized', 'timeout', 'repository not found'):
            with patch.object(m.subprocess, 'run', return_value=subprocess.CompletedProcess([], 1, '', message)):
                with self.assertRaises(RuntimeError):
                    m.registry_digest('image:version')
        with patch.object(m.subprocess, 'run', return_value=subprocess.CompletedProcess([], 1, '', 'manifest unknown')):
            self.assertIsNone(m.registry_digest('image:version'))
        with patch.object(m.subprocess, 'run', return_value=subprocess.CompletedProcess([], 1, '', 'ERROR: image:version: not found\n')):
            self.assertIsNone(m.registry_digest('image:version'))

    def test_different_release_digest_refused(self):
        with patch.object(m, 'registry_digest', return_value=OTHER), self.assertRaises(ValueError):
            m.preflight({'api': {'image': 'image', 'digest': DIGEST}}, 'v0.2.4')

    def test_shared_and_build_dependencies_trigger_workflows(self):
        # Inspect the actual filters, not a duplicate list pretending to execute Actions.
        for name in ('ci', 'publish-images'):
            text = (ROOT / f'.github/workflows/{name}.yml').read_text()
            paths = text.split('  pull_request:', 1)[1].split('\n\n', 1)[0]
            for path in ('shared/**', 'api/**', 'media/**', 'web/**', 'docker/**',
                         '.dockerignore', 'Cargo.toml', 'Cargo.lock', 'rust-toolchain.toml',
                         '.github/workflows/**', 'deploy/compose/**'):
                self.assertIn('      - ' + path + '\n', paths + '\n')
        text = (ROOT / '.github/workflows/ci.yml').read_text()
        self.assertNotIn('paths:', text.split('  push:', 1)[1].split('concurrency:', 1)[0])



class Promotion(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.old = os.getcwd()
        os.chdir(self.tmp.name)
        Path('Cargo.toml').write_text('[workspace.package]\nversion="0.2.4"\n')
        Path('digests').mkdir()
        self.images = {}
        for name in m.SERVICES:
            item = dict(image=f'ghcr.io/{REPO}/{name}', digest=DIGEST, revision=SHA, tag='sha-build')
            self.images[name] = item
            Path(f'digests/{name}.json').write_text(json.dumps(item))
        self.env = patch.dict(os.environ, GITHUB_REPOSITORY=REPO, SOURCE_SHA=SHA,
                              CI_RUN_ID='1', GITHUB_RUN_ID='2')
        self.env.start()
        self.api = patch.object(m, 'api', side_effect=lambda path: ci() if path.startswith('actions/') else {'object': {'sha': SHA}})
        self.api.start()

    def tearDown(self):
        self.api.stop()
        self.env.stop()
        os.chdir(self.old)
        self.tmp.cleanup()

    def test_shipped_version_only_records_candidate(self):
        with patch.object(m, 'tag_sha', return_value='d' * 40), patch.object(m, 'registry_digest', return_value=DIGEST), patch.object(m, 'run') as writes:
            m.promote()
            writes.assert_not_called()
        manifest = json.loads(Path('image-set.json').read_text())
        self.assertFalse(manifest['promoted'])
        self.assertEqual(manifest['revision'], SHA)
        env = Path('image-set.env').read_text()
        self.assertEqual(env.count('@sha256:'), 4)

    def test_failure_does_not_write_any_release_tags(self):
        with patch.object(m, 'api', return_value=ci(conclusion='failure')), patch.object(m, 'run') as writes:
            with self.assertRaises(ValueError):
                m.promote()
            writes.assert_not_called()
        self.assertFalse(Path('image-set.json').exists())

    def test_preflight_whole_set_before_any_write(self):
        def digest(ref):
            return OTHER if ref.endswith('/media:v0.2.4') else DIGEST
        with patch.object(m, 'tag_sha', return_value=None), patch.object(m, 'registry_digest', side_effect=digest), patch.object(m, 'run') as writes:
            with self.assertRaises(ValueError):
                m.promote()
            writes.assert_not_called()

    def test_successful_set_preserves_exact_digest(self):
        with patch.object(m, 'tag_sha', return_value=None), patch.object(m, 'registry_digest', return_value=DIGEST), patch.object(m, 'run') as writes:
            m.promote()
            self.assertEqual(writes.call_count, 3)
            for call in writes.call_args_list:
                self.assertIn('--prefer-index=false', call.args)
                self.assertTrue(call.args[-1].endswith('@' + DIGEST))
        self.assertTrue(json.loads(Path('image-set.json').read_text())['promoted'])

    def test_release_refuses_candidate_before_external_write(self):
        Path('image-set.json').write_text(json.dumps(dict(promoted=False, revision=SHA, version='v0.2.4')))
        with patch.dict(os.environ, RELEASE_TAG='v0.2.4'), patch.object(m, 'run') as writes:
            with self.assertRaises(ValueError):
                m.release()
            writes.assert_not_called()

    def prepare_release(self):
        Path('image-set.json').write_text(json.dumps(dict(
            promoted=True, revision=SHA, version='v0.2.4', publish_run_id=2,
            ci_run_id=1, images=self.images)))
        self.publish = dict(conclusion='success', status='completed', event='workflow_run',
                            head_branch='main', repository={'full_name': REPO},
                            path='.github/workflows/publish-images.yml')

    def release_api(self, path):
        if path == 'actions/runs/2':
            return self.publish
        if path == 'actions/runs/1':
            return ci()
        if path.startswith('releases'):
            return []
        return {'object': {'sha': SHA}}

    def test_release_requires_successful_publisher(self):
        self.prepare_release()
        self.publish['conclusion'] = 'failure'
        with patch.dict(os.environ, RELEASE_TAG='v0.2.4'), patch.object(m, 'api', side_effect=self.release_api), patch.object(m, 'run') as writes:
            with self.assertRaises(ValueError):
                m.release()
            writes.assert_not_called()

    def test_release_rejects_foreign_tag_or_digest(self):
        self.prepare_release()
        for tag, digest in [('d' * 40, DIGEST), (SHA, OTHER)]:
            with patch.dict(os.environ, RELEASE_TAG='v0.2.4'), patch.object(m, 'api', side_effect=self.release_api), patch.object(m, 'tag_sha', return_value=tag), patch.object(m, 'registry_digest', return_value=digest), patch.object(m, 'run', return_value=SHA) as writes:
                with self.assertRaises(ValueError):
                    m.release()
                self.assertEqual(writes.call_args_list[0].args, ('git', 'rev-parse', 'HEAD'))
                self.assertEqual(writes.call_count, 1)

    def test_verified_release_attaches_exact_manifest(self):
        self.prepare_release()
        with patch.dict(os.environ, RELEASE_TAG='v0.2.4'), patch.object(m, 'api', side_effect=self.release_api), patch.object(m, 'tag_sha', return_value=None), patch.object(m, 'registry_digest', return_value=DIGEST), patch.object(m, 'run', return_value=SHA) as writes:
            m.release()
            args = writes.call_args_list[-1].args
            self.assertEqual(args[:4], ('gh', 'release', 'create', 'v0.2.4'))
            self.assertIn('image-set.json', args)
            self.assertIn('image-set.env', args)
            self.assertIn(SHA, args)


if __name__ == '__main__':
    unittest.main()
