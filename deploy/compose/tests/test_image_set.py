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
        install = workflow.split('      - name: Install Caddy\n', 1)[1].split('\n      - name:', 1)[0]
        # The download API can return a newer version despite its version query.
        # Use the exact upstream release asset and verify the installed binary.
        asset = f'https://github.com/caddyserver/caddy/releases/download/v{version}/caddy_{version}_linux_amd64.tar.gz'
        self.assertIn(f'curl -fsSL "{asset}" -o "$caddy_archive"', install)
        self.assertNotIn('caddyserver.com/api/download', install)
        self.assertIn('tar -xzf "$caddy_archive" -C /usr/local/bin caddy', install)
        self.assertIn('test "$(caddy version | cut -d \' \' -f 1)" = "v' + version + '"', install)

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
            ci_run_id=1, images=self.images,
            minio=dict(image=f'ghcr.io/{REPO}/minio', digest=DIGEST, source_pin=m.MINIO_PIN))))
        self.publish = dict(conclusion='success', status='completed', event='workflow_run',
                            head_branch='main', head_sha=SHA, repository={'full_name': REPO},
                            path='.github/workflows/publish-images.yml')

    def release_api(self, path):
        if path == 'actions/runs/2':
            return self.publish
        if path == 'actions/runs/1':
            return ci()
        if path == 'releases/latest':
            return dict(tag_name='v0.2.4', draft=False, prerelease=False, published_at='2026-09-29T00:00:00Z')
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
        with patch.dict(os.environ, RELEASE_TAG='v0.2.4'), patch.object(m, 'api', side_effect=self.release_api), patch.object(m, 'tag_sha', side_effect=[None, SHA]), patch.object(m, 'registry_digest', return_value=DIGEST), patch.object(m, 'run', return_value=SHA) as writes:
            m.release()
            args = next(call.args for call in writes.call_args_list if call.args[:3] == ('gh', 'release', 'create'))
            self.assertEqual(args[:4], ('gh', 'release', 'create', 'v0.2.4'))
            self.assertIn('image-set.json', args)
            self.assertIn('image-set.env', args)
            self.assertIn(SHA, args)
            alias_calls = [call.args for call in writes.call_args_list if call.args[0] == 'docker']
            self.assertEqual(len(alias_calls), 4)
            for call in alias_calls:
                self.assertTrue(call[-2].endswith(':latest'))
                self.assertTrue(call[-1].endswith('@' + DIGEST))

    def latest_run(self, api=None, digest=None, run=None, tag=SHA):
        with patch.dict(os.environ, RELEASE_TAG='v0.2.4'), patch.object(m, 'api', side_effect=api or self.release_api), patch.object(m, 'tag_sha', return_value=tag), patch.object(m, 'registry_digest', side_effect=digest or (lambda ref: DIGEST)), patch.object(m, 'run', side_effect=run or (lambda *args: SHA)) as calls:
            m.latest()
            return [c.args for c in calls.call_args_list]

    def test_latest_repairs_published_release_after_main_advanced_without_new_release(self):
        self.prepare_release()
        manifest = json.loads(Path('image-set.json').read_text())
        expected = {}
        for index, (name, item) in enumerate({**manifest['images'], 'minio': manifest['minio']}.items(), 1):
            item['digest'] = 'sha256:' + str(index) * 64
            expected[item['image']] = item['digest']
        Path('image-set.json').write_text(json.dumps(manifest))
        calls = self.latest_run(digest=lambda ref: expected[ref.rsplit(':', 1)[0]])
        self.assertIn(('git', 'merge-base', '--is-ancestor', SHA, 'HEAD'), calls)
        self.assertIn(('git', 'fetch', '--no-tags', 'origin', 'refs/tags/v0.2.4:refs/tags/v0.2.4'), calls)
        self.assertEqual(len([c for c in calls if c[0] == 'docker']), 4)
        self.assertFalse(any(c[0] == 'gh' for c in calls))
        for call in (c for c in calls if c[0] == 'docker'):
            image = call[-2].removesuffix(':latest')
            self.assertEqual(call[-1], image + '@' + expected[image])

    def test_existing_release_is_preserved_while_aliases_are_repaired(self):
        self.prepare_release()
        def api(path):
            return [{'tag_name': 'v0.2.4'}] if path == 'releases?per_page=100' else self.release_api(path)
        with patch.dict(os.environ, RELEASE_TAG='v0.2.4'), patch.object(m, 'api', side_effect=api), patch.object(m, 'tag_sha', return_value=SHA), patch.object(m, 'registry_digest', return_value=DIGEST), patch.object(m, 'run', return_value=SHA) as calls:
            m.release()
            self.assertFalse(any(c.args[0] == 'gh' for c in calls.call_args_list))
            self.assertEqual(len([c for c in calls.call_args_list if c.args[0] == 'docker']), 4)

    def test_latest_refuses_old_draft_and_prerelease_before_alias_writes(self):
        self.prepare_release()
        for changes in ({'tag_name': 'v0.2.5'}, {'draft': True}, {'prerelease': True}, {'published_at': None}):
            calls = []
            def api(path):
                value = self.release_api(path)
                return {**value, **changes} if path == 'releases/latest' else value
            with self.subTest(changes=changes), self.assertRaises(ValueError):
                self.latest_run(api=api, run=lambda *args: calls.append(args))
            self.assertEqual(calls, [])

    def test_latest_preflights_minio_and_all_app_digests_before_any_alias_write(self):
        self.prepare_release()
        for bad in ('media:v0.2.4', 'minio:' + m.MINIO_PIN):
            calls = []
            with self.subTest(bad=bad), self.assertRaises(ValueError):
                self.latest_run(digest=lambda ref: OTHER if ref.endswith(bad) else DIGEST,
                                run=lambda *args: calls.append(args))
            self.assertFalse(any(c[0] == 'docker' for c in calls))

    def test_latest_refuses_wrong_publish_source_or_tag(self):
        self.prepare_release()
        self.publish['head_sha'] = 'd' * 40
        calls = []
        with self.assertRaises(ValueError):
            self.latest_run(run=lambda *args: calls.append(args))
        self.assertFalse(any(c[0] == 'docker' for c in calls))
        self.publish['head_sha'] = SHA
        with self.assertRaises(ValueError):
            self.latest_run(tag='d' * 40)

    def test_latest_partial_write_failure_remains_red_and_is_retryable(self):
        self.prepare_release()
        calls = []
        def fail(*args):
            calls.append(args)
            if args[0] == 'docker' and args[-2].endswith('/web:latest'):
                raise subprocess.CalledProcessError(1, args)
            return SHA
        with self.assertRaises(subprocess.CalledProcessError):
            self.latest_run(run=fail)
        self.assertEqual(len([c for c in calls if c[0] == 'docker']), 2)
        self.assertEqual(len([c for c in self.latest_run() if c[0] == 'docker']), 4)

    def test_latest_verifies_written_alias_digest(self):
        self.prepare_release()
        with self.assertRaises(ValueError):
            self.latest_run(digest=lambda ref: OTHER if ref.endswith(':latest') else DIGEST)

    def test_failed_release_creation_never_updates_latest(self):
        self.prepare_release()
        def run(*args):
            if args[0] == 'gh':
                raise subprocess.CalledProcessError(1, args)
            return SHA
        with patch.dict(os.environ, RELEASE_TAG='v0.2.4'), patch.object(m, 'api', side_effect=self.release_api), patch.object(m, 'tag_sha', return_value=None), patch.object(m, 'registry_digest', return_value=DIGEST), patch.object(m, 'run', side_effect=run) as calls:
            with self.assertRaises(subprocess.CalledProcessError):
                m.release()
            self.assertFalse(any(c.args[0] == 'docker' for c in calls.call_args_list))


if __name__ == '__main__':
    unittest.main()
