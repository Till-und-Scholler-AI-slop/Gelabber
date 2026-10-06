"""Fast safety/false-positive checks for the slow local restore drill."""
import argparse
import http.server
import importlib.util
import json
from pathlib import Path
import shutil
import subprocess
import tempfile
import threading
import unittest
from unittest import mock


SPEC = importlib.util.spec_from_file_location('restore_drill', Path(__file__).parents[1] / 'check-upgrade-storage-restore.py')
drill = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(drill)


class SafetyTests(unittest.TestCase):
    def args(self, output):
        return argparse.Namespace(output=output, target_dir=None, old_ref='v0.3.1', current_ref='HEAD')

    def test_existing_artifacts_are_never_overwritten(self):
        with tempfile.TemporaryDirectory() as tmp:
            output = Path(tmp)
            (output / 'report.json').write_text('keep')
            with self.assertRaises(FileExistsError):
                drill.Drill(self.args(output))
            self.assertEqual((output / 'report.json').read_text(), 'keep')

    @unittest.skipUnless(Path('/proc').exists() and shutil.which('sleep'), 'Linux process identity guard')
    def test_shared_cache_refuses_to_replace_a_running_executable(self):
        with tempfile.TemporaryDirectory() as tmp:
            binary = Path(tmp) / 'gelabber-api'
            shutil.copy2(shutil.which('sleep'), binary)
            process = subprocess.Popen([str(binary), '30'])
            try:
                with self.assertRaisesRegex(ValueError, 'running API binary'):
                    drill.guard_running_binary(binary)
            finally:
                process.terminate()
                process.wait(timeout=3)
            drill.guard_running_binary(binary)

    def test_remote_daemon_and_environment_override_rejected_before_resources(self):
        for env, endpoint in [({}, 'ssh://prod'), ({'DOCKER_HOST': 'unix:///tmp/other'}, 'unix:///var/run/docker.sock'),
                              ({'DOCKER_CONTEXT': 'prod'}, 'unix:///var/run/docker.sock')]:
            with self.subTest(env=env), tempfile.TemporaryDirectory() as tmp:
                instance = drill.Drill(self.args(Path(tmp) / 'artifact'))
                with mock.patch.dict('os.environ', env, clear=True), mock.patch.object(instance, 'docker') as docker:
                    docker.return_value = json.dumps([{'Endpoints': {'docker': {'Host': endpoint}}}]).encode()
                    with self.assertRaises(ValueError):
                        instance.preflight()
                    self.assertFalse(any(call.args[0] in ('create', 'run', 'volume') for call in docker.call_args_list))

    def test_cleanup_refuses_resources_with_a_foreign_nonce(self):
        with tempfile.TemporaryDirectory() as tmp:
            instance = drill.Drill(self.args(Path(tmp) / 'artifact'))
            instance.containers, instance.volumes = ['foreign-container'], ['foreign-volume']
            with mock.patch.object(instance, 'docker') as docker:
                docker.side_effect = [json.dumps([{'Config': {'Labels': {drill.LABEL: 'other'}}}]).encode(),
                                      json.dumps([{'Labels': {drill.LABEL: 'other'}}]).encode()]
                instance.cleanup()
                self.assertEqual([call.args[0] for call in docker.call_args_list], ['inspect', 'volume'])
                self.assertEqual(docker.call_args_list[1].args[1], 'inspect')
                self.assertEqual(len(instance.report['cleanup_errors']), 2)
                self.assertFalse(instance.report['passed'])

    def test_every_archive_hash_checked_before_any_restore_mutation(self):
        for changed in ('database', 'objects'):
            with self.subTest(changed=changed), tempfile.TemporaryDirectory() as tmp:
                instance = drill.Drill(self.args(Path(tmp) / 'artifact'))
                snapshot = {'label': 'fixture', 'paths': {}, 'manifest': {}}
                for key in ('database', 'objects'):
                    path = instance.output / key
                    path.write_bytes(b'original')
                    snapshot['paths'][key] = path
                    snapshot['manifest'][f'{key}_sha256'] = drill.sha(b'original')
                snapshot['paths'][changed].write_bytes(b'modified')
                with mock.patch.object(instance, 'docker') as docker:
                    with self.assertRaises(AssertionError):
                        instance.restore_database(snapshot, 'never_created')
                    docker.assert_not_called()

    def test_snapshot_cannot_restore_with_another_minio_image(self):
        with tempfile.TemporaryDirectory() as tmp:
            instance = drill.Drill(self.args(Path(tmp) / 'artifact'))
            instance.report['images']['minio'] = {'id': 'sha256:now'}
            paths = {key: instance.output / key for key in ('database', 'objects')}
            for path in paths.values():
                path.write_bytes(b'ok')
            snapshot = {'label': 'fixture', 'paths': paths, 'manifest': {
                'database_sha256': drill.sha(b'ok'), 'objects_sha256': drill.sha(b'ok'), 'minio_image_id': 'sha256:old'}}
            with self.assertRaisesRegex(AssertionError, 'same MinIO restore image'):
                instance.verified_snapshot(snapshot)

    def test_fixture_cannot_follow_an_arbitrary_presigned_url(self):
        valid = 'http://127.0.0.1:43210/bucket/object?signature=fixture'
        self.assertEqual(drill.loopback_url(valid, 43210), valid)
        for url in ['http://127.0.0.1:43211/x', 'http://example.test:43210/x',
                    'https://127.0.0.1:43210/x', 'http://user:pw@127.0.0.1:43210/x']:
            with self.subTest(url=url), self.assertRaises(ValueError):
                drill.loopback_url(url, 43210)

    def test_negative_control_rejects_unrelated_error_and_false_pass(self):
        def failure():
            raise AssertionError('missing object: 404')
        self.assertEqual(drill.expected_failure(failure, 'missing object:'), 'missing object: 404')
        with self.assertRaisesRegex(AssertionError, 'missing object'):
            drill.expected_failure(failure, 'bytes hash:')
        with self.assertRaisesRegex(AssertionError, 'unexpectedly accepted'):
            drill.expected_failure(lambda: None, 'missing object:')

    def test_api_redirect_is_explicit_and_object_request_has_no_session_cookie(self):
        requests = []

        class Handler(http.server.BaseHTTPRequestHandler):
            def do_GET(self):
                requests.append((self.path, self.headers.get('Cookie')))
                if self.path == '/api/attachments/fixture':
                    self.send_response(307)
                    self.send_header('Location', f'http://127.0.0.1:{self.server.server_port}/blob')
                    self.end_headers()
                else:
                    self.send_response(200)
                    self.end_headers()
                    self.wfile.write(b'fixture')

            def log_message(self, *args):
                pass

        server = http.server.ThreadingHTTPServer(('127.0.0.1', 0), Handler)
        worker = threading.Thread(target=server.serve_forever, daemon=True)
        worker.start()
        try:
            client = drill.Client(server.server_port)
            client.cookies = {'gelabber_session': 'fixture-session'}
            status, headers, _ = client.raw('/api/attachments/fixture')
            self.assertEqual(status, 307)
            self.assertEqual(len(requests), 1)
            self.assertEqual(drill.http_request(headers['Location'])[2], b'fixture')
            self.assertEqual(requests, [('/api/attachments/fixture', 'gelabber_session=fixture-session'), ('/blob', None)])
            with self.assertRaises(ValueError):
                client.raw('//other-origin/path')
        finally:
            server.shutdown()
            server.server_close()
            worker.join(timeout=3)


if __name__ == '__main__':
    unittest.main()
