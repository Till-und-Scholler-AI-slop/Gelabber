"""Real Compose config validation; no Docker daemon, pulls, or rollout required."""
import json
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]


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
                       '--env-file', str(ROOT / example), '--env-file', str(env),
                       '-f', str(ROOT / 'compose.yaml')]
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
