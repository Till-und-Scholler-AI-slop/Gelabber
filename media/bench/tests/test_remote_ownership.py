import contextlib
import importlib.util
import io
import json
from pathlib import Path
import subprocess
import sys
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location('remote_runner', Path(__file__).resolve().parents[1] / 'run-remote.py')
runner = importlib.util.module_from_spec(spec)
spec.loader.exec_module(runner)


class RemoteOwnershipTests(unittest.TestCase):
    def namespace(self):
        namespace = {'__name__': 'test_remote'}
        config = {'group': 'gelabber-bench-testfixture', 'op': 'clock'}
        with patch('sys.stdin', io.StringIO(json.dumps(config))), contextlib.redirect_stdout(io.StringIO()):
            exec(compile(runner.REMOTE, 'remote-helper', 'exec'), namespace)
        return namespace

    def test_existing_foreign_container_is_never_owned(self):
        namespace = self.namespace()
        inspect = subprocess.CompletedProcess([], 0, json.dumps([{'Config': {'Labels': {'gelabber.bench.run': 'other'}}}]), '')
        with patch('subprocess.run', return_value=inspect):
            with self.assertRaisesRegex(AssertionError, 'refusing foreign'):
                namespace['owned']('gelabber-bench-testfixture-engine')
            with self.assertRaises(AssertionError):
                namespace['owned']('production-media')

    def test_owned_label_and_exact_name_are_both_required(self):
        namespace = self.namespace()
        inspect = subprocess.CompletedProcess([], 0, json.dumps([{'Config': {'Labels': {'gelabber.bench.run': 'gelabber-bench-testfixture'}}}]), '')
        with patch('subprocess.run', return_value=inspect):
            self.assertIsNotNone(namespace['owned']('gelabber-bench-testfixture-engine'))
            self.assertIsNotNone(namespace['owned']('gelabber-bench-testfixture-redis'))
            with self.assertRaises(AssertionError):
                namespace['owned']('production-redis')

    def test_default_plan_does_not_run_ssh_or_mutate_docker(self):
        argv = ['run-remote.py', '--ssh-host', 'root@test.invalid', '--ssh-control', '/test/control',
                '--server-ip', '192.0.2.1', '--output', '/test/output']
        def output(command, **kwargs):
            if command[:3] == ['docker', 'image', 'inspect']:
                return json.dumps([{'Id': 'sha256:test', 'Size': 100, 'Config': {'Labels': {}}}])
            if command[:3] == ['git', 'rev-parse', 'HEAD']:
                return 'revision\n'
            if command[:3] == ['git', 'status', '--porcelain']:
                return ''
            raise AssertionError('unplanned command: ' + repr(command))
        with patch('sys.argv', argv), patch('subprocess.check_output', side_effect=output), patch('subprocess.run') as mutation, contextlib.redirect_stdout(io.StringIO()):
            self.assertEqual(runner.main(), 0)
            mutation.assert_not_called()

    def test_cleanup_evidence_redacts_token_and_never_exports_environment(self):
        config = {'group': 'gelabber-bench-testfixture', 'op': 'cleanup', 'token': 'private-test-token'}
        state = {'Status': 'exited', 'ExitCode': 137, 'OOMKilled': True, 'StartedAt': 'start', 'FinishedAt': 'finish'}
        info = {'Config': {'Labels': {'gelabber.bench.run': config['group']}, 'Env': ['PRIVATE=secret']}, 'State': state}
        inspect = subprocess.CompletedProcess([], 0, json.dumps([info]), '')
        output = io.StringIO()
        with patch('sys.stdin', io.StringIO(json.dumps(config))), patch('subprocess.run', return_value=inspect), \
             patch('subprocess.check_output', return_value='log private-test-token'), contextlib.redirect_stdout(output):
            exec(compile(runner.REMOTE, 'remote-helper', 'exec'), {'__name__': 'test_remote'})
        self.assertNotIn('private-test-token', output.getvalue())
        self.assertNotIn('PRIVATE', output.getvalue())
        self.assertEqual(json.loads(output.getvalue())[config['group'] + '-engine']['state'], state)


if __name__ == '__main__':
    unittest.main()
