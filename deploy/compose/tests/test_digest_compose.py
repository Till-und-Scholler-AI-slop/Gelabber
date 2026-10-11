"""Real Compose config validation; no Docker daemon, pulls, or rollout required."""
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]

# The examples ship without passwords (#188); an operator fills them in.
SECRETS = ''.join(f'{key}=test-{key.lower()}\n' for key in (
    'POSTGRES_PASSWORD', 'MINIO_ROOT_PASSWORD', 'TURN_PASSWORD'))


def secrets_env(tmp):
    path = Path(tmp) / 'secrets.env'
    path.write_text(SECRETS)
    return path


@unittest.skipUnless(shutil.which('docker'), 'Docker CLI is unavailable')
class DigestCompose(unittest.TestCase):
    def check_roundtrip(self, example, homelab=False):
        with tempfile.TemporaryDirectory() as tmp:
            configs = []
            for letter in ('b', 'c'):
                env = Path(tmp) / f'{letter}.env'
                env.write_text(''.join(
                    f'GELABBER_{service.upper()}_IMAGE=ghcr.io/till-und-scholler-ai-slop/gelabber/{service}@sha256:{letter * 64}\n'
                    for service in ('api', 'web', 'media', 'minio')))
                cmd = ['docker', 'compose', '--project-directory', str(ROOT),
                       '--env-file', str(ROOT / example), '--env-file', str(secrets_env(tmp)),
                       '--env-file', str(env), '-f', str(ROOT / 'compose.yaml')]
                if homelab:
                    cmd += ['-f', str(ROOT / 'compose.homelab.yaml')]
                subprocess.run(cmd + ['config', '-q'], check=True)
                config = json.loads(subprocess.check_output(cmd + ['config', '--format', 'json'], text=True))
                for service in ('api', 'web', 'media', 'minio'):
                    self.assertEqual(config['services'][service]['image'],
                                     f'ghcr.io/till-und-scholler-ai-slop/gelabber/{service}@sha256:{letter * 64}')
                configs.append(config)
            self.assertEqual(configs[0]['volumes'], configs[1]['volumes'])
            for service in ('postgres', 'minio'):
                self.assertEqual(configs[0]['services'][service]['volumes'],
                                 configs[1]['services'][service]['volumes'])

    def test_base_digest_rollout_and_rollback_configuration(self):
        self.check_roundtrip('.env.example')

    def test_homelab_digest_rollout_and_rollback_configuration(self):
        self.check_roundtrip('.env.homelab.example', homelab=True)

    def test_turn_default_offers_udp_and_tcp_on_the_configured_public_port(self):
        with tempfile.TemporaryDirectory() as tmp:
            env = Path(tmp) / 'turn.env'
            env.write_text(SECRETS + 'TURN_PUBLIC_HOST=voice.example.test\nTURN_PORT=3479\n')
            cmd = ['docker', 'compose', '--project-directory', str(ROOT),
                   '--env-file', str(env), '-f', str(ROOT / 'compose.yaml')]
            config = json.loads(subprocess.check_output(cmd + ['config', '--format', 'json'], text=True))
            self.assertEqual(config['services']['api']['environment']['TURN_URLS'].split(','),
                             ['stun:voice.example.test:3479',
                              'turn:voice.example.test:3479?transport=udp',
                              'turn:voice.example.test:3479?transport=tcp'])
            ports = config['services']['coturn']['ports']
            self.assertEqual({p['protocol'] for p in ports
                              if p['target'] == 3478 and str(p['published']) == '3479'},
                             {'udp', 'tcp'})


@unittest.skipUnless(shutil.which('docker'), 'Docker CLI is unavailable')
class SecureDefaults(unittest.TestCase):
    """#188: no shipped passwords, plain HTTP only on this machine."""

    def config(self, env_text):
        with tempfile.TemporaryDirectory() as tmp:
            env = Path(tmp) / 'test.env'
            env.write_text(env_text)
            cmd = ['docker', 'compose', '--project-directory', str(ROOT),
                   '--env-file', str(env), '-f', str(ROOT / 'compose.yaml'),
                   'config', '--format', 'json']
            # The shell's environment wins over --env-file (CI exports
            # MINIO_ROOT_PASSWORD for its own MinIO); test the file alone.
            environ = {key: value for key, value in os.environ.items()
                       if not key.endswith('PASSWORD')}
            return subprocess.run(cmd, capture_output=True, text=True, env=environ)

    def test_every_password_is_required(self):
        for key in ('POSTGRES_PASSWORD', 'MINIO_ROOT_PASSWORD', 'TURN_PASSWORD'):
            missing = ''.join(line + '\n' for line in SECRETS.splitlines()
                              if not line.startswith(key + '='))
            for env_text in (missing, missing + f'{key}=\n'):
                result = self.config(env_text)
                self.assertNotEqual(result.returncode, 0, key)
                self.assertIn(key, result.stderr)

    def test_plain_http_entry_binds_loopback(self):
        result = self.config(SECRETS)
        self.assertEqual(result.returncode, 0, result.stderr)
        ports = json.loads(result.stdout)['services']['proxy']['ports']
        self.assertEqual([p['host_ip'] for p in ports], ['127.0.0.1'])
        self.assertEqual(json.loads(result.stdout)['services']['api']['environment']['API_COOKIE_SECURE'], 'false')

    def test_examples_ship_no_passwords(self):
        for example in ('.env.example', '.env.homelab.example'):
            lines = (ROOT / example).read_text().splitlines()
            values = dict(line.split('=', 1) for line in lines
                          if '=' in line and not line.lstrip().startswith('#'))
            for key, value in values.items():
                if key.endswith('PASSWORD'):
                    self.assertEqual(value, '', f'{example}: {key}')
